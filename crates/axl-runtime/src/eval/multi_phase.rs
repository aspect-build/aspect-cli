use std::collections::HashMap;
use std::path::{Path, PathBuf};

use anyhow::anyhow;
use starlark::environment::{FrozenModule, Module};
use starlark::eval::Evaluator;
use starlark::values::list::AllocList;
use starlark::values::{Heap, Value, ValueLike};
use uuid::Uuid;

use crate::engine::bazel::Bazel;
use crate::engine::cli_args::CliArgs;
use crate::engine::config::feature_context::FeatureContext;
use crate::engine::config::feature_map::{FeatureMap, construct_features};
use crate::engine::config::trait_map::TraitMap;
use crate::engine::config::{ConfigContext, ConfiguredTask};
use crate::engine::task::FrozenTask;
use crate::engine::task_context::TaskContext;
use crate::engine::task_info::TaskInfo;
use crate::engine::types::feature::{
    FeatureInstance, extract_feature_impl_fn, extract_feature_type_id, populate_feature_custom_args,
};
use crate::engine::types::r#trait::extract_trait_type_id;
use crate::eval::error::EvalError;
use crate::eval::load::{AxlLoader, ModuleScope};
use crate::eval::task::FrozenTaskModuleLike;

/// Wrapper around a live Starlark Module heap.
///
/// All three evaluation phases share this heap so `Value<'v>` references
/// are valid across phase boundaries without freeze/thaw cycles.
pub struct ModuleEnv<'v>(pub(crate) Module<'v>);

impl<'v> ModuleEnv<'v> {
    /// Run a closure with a fresh module heap, hiding the heap lifetime from the caller.
    pub fn with<R, E>(func: impl for<'a> FnOnce(&ModuleEnv<'a>) -> Result<R, E>) -> Result<R, E> {
        Module::with_temp_heap(|m| func(&ModuleEnv(m)))
    }

    pub fn heap(&self) -> Heap<'v> {
        self.0.heap()
    }
}

/// Spec for a module's exported tasks and features, consumed by Phase 1.
pub struct ModuleTaskSpec {
    pub name: String,
    pub root: PathBuf,
    /// Map from absolute task file path → (label, exported symbols)
    pub use_tasks: HashMap<PathBuf, (String, Vec<String>)>,
    /// List of (absolute feature file path, symbol name)
    pub use_features: Vec<(PathBuf, String)>,
}

/// Multi-phase Starlark evaluator with one shared Module heap.
///
/// Phases:
/// 1. `eval`              — load task scripts → `Vec<Value<'v>>` (ConfiguredTask on shared heap)
/// 2. `eval_features`     — construct feature instances onto the shared heap
/// 3. `eval_config`       — run config.axl files; mutations visible via shared heap
/// 4. `eval_feature_impls`— run enabled feature implementations
/// 5. `execute_with_args` — call the selected task implementation
pub struct MultiPhaseEval<'v, 'loader> {
    env: &'loader ModuleEnv<'v>,
    loader: &'loader AxlLoader,
    /// Feature type values on the shared heap, collected during Phase 1.
    /// Tuple: (type_id, feature_type_value, source_path)
    feature_types: Vec<(u64, Value<'v>, PathBuf)>,
    /// Global feature map allocated on the shared heap during Phase 2.
    feature_map_value: Option<Value<'v>>,
    /// Global trait map allocated on the shared heap during Phase 3.
    trait_map_value: Option<Value<'v>>,
}

impl<'v, 'loader> MultiPhaseEval<'v, 'loader> {
    pub fn new(env: &'loader ModuleEnv<'v>, loader: &'loader AxlLoader) -> Self {
        MultiPhaseEval {
            env,
            loader,
            feature_types: Vec::new(),
            feature_map_value: None,
            trait_map_value: None,
        }
    }

    fn heap(&self) -> Heap<'v> {
        self.env.0.heap()
    }

    /// Load and evaluate an AXL file via AxlLoader (per-file frozen module).
    fn eval_file(&self, scope: &ModuleScope, abs_path: &Path) -> Result<FrozenModule, EvalError> {
        self.loader.module_stack.borrow_mut().push(scope.clone());
        let frozen = self.loader.eval_module(abs_path)?;
        self.loader.module_stack.borrow_mut().pop();
        self.loader
            .cache_module(abs_path.to_path_buf(), frozen.clone());
        Ok(frozen)
    }

    /// Phase 1: evaluate task scripts and module task specs.
    ///
    /// Each task file is loaded into its own frozen module (via `AxlLoader::eval_module`),
    /// then `ConfiguredTask` instances are allocated on the shared heap and returned as
    /// live `Value<'v>` references. Feature type values are collected for Phase 2.
    pub fn eval(
        &mut self,
        scripts: &[PathBuf],
        root_scope: ModuleScope,
        modules: Vec<ModuleTaskSpec>,
    ) -> Result<Vec<Value<'v>>, EvalError> {
        let mut task_values: Vec<Value<'v>> = Vec::new();
        let heap = self.heap();

        // Evaluate auto-discovered AXL scripts (axl_sources in repo root)
        for path in scripts {
            let frozen = self.eval_file(&root_scope, path)?;
            for symbol in frozen.tasks() {
                let configured =
                    ConfiguredTask::from_frozen_module(&frozen, &symbol, path.clone())?;
                task_values.push(heap.alloc(configured));
            }
        }

        // Evaluate module specs (use_task and use_feature entries from MODULE.aspect)
        for spec in modules {
            let scope = ModuleScope {
                name: spec.name.clone(),
                path: spec.root.clone(),
            };

            for (abs_path, (label, symbols)) in spec.use_tasks {
                let frozen = self.eval_file(&scope, &abs_path)?;
                for symbol in &symbols {
                    if frozen.has_name(symbol) {
                        if !frozen.has_task(symbol) {
                            return Err(EvalError::UnknownError(anyhow!(
                                "invalid use_task({:?}, {:?}) in @{} module",
                                label,
                                symbol,
                                spec.name
                            )));
                        }
                        let configured =
                            ConfiguredTask::from_frozen_module(&frozen, symbol, abs_path.clone())?;
                        task_values.push(heap.alloc(configured));
                    } else {
                        return Err(EvalError::UnknownError(anyhow!(
                            "task symbol {:?} not found in @{} module use_task({:?})",
                            symbol,
                            spec.name,
                            label
                        )));
                    }
                }
            }

            for (abs_path, symbol) in spec.use_features {
                let frozen = self.eval_file(&scope, &abs_path)?;
                let owned = frozen
                    .get(symbol.as_str())
                    .map_err(|_| EvalError::MissingSymbol(symbol.clone()))?;
                let type_id = extract_feature_type_id(owned.value()).ok_or_else(|| {
                    EvalError::UnknownError(anyhow!(
                        "symbol {:?} in {:?} is not a feature type",
                        symbol,
                        abs_path
                    ))
                })?;
                // access_owned_frozen_value registers the frozen heap as a dependency of the
                // live heap (keeping it alive) and returns Value<'v>.
                let feature_val = heap.access_owned_frozen_value(&owned);
                self.feature_types
                    .push((type_id, feature_val, abs_path.clone()));
            }
        }

        Ok(task_values)
    }

    /// Returns feature types with their source paths, collected during Phase 1.
    /// Each entry is `(type_id, feature_type_value, source_path)`.
    /// Used by the CLI layer to build per-feature arg specs and help headings.
    pub fn feature_types_with_paths(&self) -> &[(u64, Value<'v>, PathBuf)] {
        &self.feature_types
    }

    /// Phase 2: construct feature instances from the feature types collected in Phase 1.
    ///
    /// Allocates a `FeatureMap` on the shared heap so Phase 3 can reference and mutate it.
    pub fn eval_features(&mut self) -> Result<(), EvalError> {
        let heap = self.heap();
        // construct_features needs (id, value) pairs; strip the PathBuf.
        let id_val_pairs: Vec<(u64, Value<'v>)> = self
            .feature_types
            .iter()
            .map(|(id, val, _)| (*id, *val))
            .collect();
        let feature_map = {
            let store = self.loader.new_store(self.loader.repo_root.clone());
            let mut eval = Evaluator::new(&self.env.0);
            eval.set_loader(self.loader);
            eval.extra = Some(&store);
            construct_features(&id_val_pairs, &mut eval)?
        };
        self.feature_map_value = Some(heap.alloc(feature_map));
        Ok(())
    }

    /// Phase 3: evaluate config files against the task values from Phase 1.
    ///
    /// Trait and feature instances are allocated on the shared heap. Config functions
    /// mutate `ConfiguredTask` values in-place via `set_attr` — the same `Value<'v>`
    /// objects returned by Phase 1 are reused, so Phase 3 sees the updated state.
    pub fn eval_config(
        &mut self,
        configs: &[PathBuf],
        tasks: &[Value<'v>],
        root_scope: &ModuleScope,
    ) -> Result<Vec<Value<'v>>, EvalError> {
        let heap = self.heap();

        // Register trait types from all tasks into a TraitMap; instances are created lazily
        // on first access (from config.axl or the task implementation).
        let trait_map = TraitMap::new();
        for task_val in tasks {
            if let Some(ct) = task_val.downcast_ref::<ConfiguredTask>() {
                if let Some(frozen_task) = ct.as_frozen_task() {
                    for trait_fv in frozen_task.traits() {
                        let trait_value: Value<'v> = trait_fv.to_value();
                        if let Some(id) = extract_trait_type_id(trait_value) {
                            trait_map.insert(id, trait_value);
                        }
                    }
                }
            }
        }
        let trait_map_value = heap.alloc(trait_map);
        self.trait_map_value = Some(trait_map_value);

        let feature_map_value = self.feature_map_value.ok_or_else(|| {
            EvalError::UnknownError(anyhow!(
                "eval_features() must be called before eval_config()"
            ))
        })?;

        // Allocate ConfigContext with the SAME Value<'v> task objects from Phase 1.
        // Mutations by config.axl are in-place on the shared heap, visible in Phase 3.
        let context_value = heap.alloc(ConfigContext::new_from_values(
            tasks.to_vec(),
            trait_map_value,
            feature_map_value,
            heap,
        ));

        // Evaluate each config file
        for config_path in configs {
            let rel_path = config_path
                .strip_prefix(&root_scope.path)
                .map_err(|e| EvalError::UnknownError(anyhow!("Failed to strip prefix: {e}")))?
                .to_path_buf();

            let abs = root_scope.path.join(&rel_path);
            let frozen = self.eval_file(root_scope, &abs)?;

            let function_name = "config";
            let def = frozen
                .get(function_name)
                .map_err(|_| EvalError::MissingSymbol(function_name.to_string()))?;
            // access_owned_frozen_value registers the frozen heap with the live heap and
            // returns Value<'v>, avoiding any lifetime extension tricks.
            let func = heap.access_owned_frozen_value(&def);

            {
                let store = self.loader.new_store(config_path.clone());
                let mut eval = Evaluator::new(&self.env.0);
                eval.set_loader(self.loader);
                eval.extra = Some(&store);
                eval.eval_function(func, &[context_value], &[])?;
            }
        }

        // Return the full task list, including any tasks added via ctx.tasks.add().
        let all_tasks = context_value
            .downcast_ref::<ConfigContext>()
            .expect("just allocated ConfigContext")
            .task_values();
        Ok(all_tasks)
    }

    /// Phase 4: run enabled feature implementations.
    ///
    /// Must be called after `eval_config` so that config files have had the opportunity
    /// to enable or disable feature instances. Each enabled feature whose type carries
    /// an `impl` function is invoked with a `FeatureContext` on the shared heap.
    ///
    /// `args_builder` is called once per enabled feature, receiving the feature's type_id
    /// and the shared heap, and returning a `Args` containing the parsed CLI values for
    /// that feature's declared args. Use `extract_feature_args` on the type value (available
    /// via `feature_types_with_paths`) to know which args belong to which feature.
    pub fn eval_feature_impls(
        &mut self,
        args_builder: impl Fn(u64, Heap<'v>) -> CliArgs<'v>,
    ) -> Result<(), EvalError> {
        let heap = self.heap();

        let feature_map_value = self.feature_map_value.ok_or_else(|| {
            EvalError::UnknownError(anyhow!(
                "eval_features() must be called before eval_feature_impls()"
            ))
        })?;
        let trait_map_value = self.trait_map_value.ok_or_else(|| {
            EvalError::UnknownError(anyhow!(
                "eval_config() must be called before eval_feature_impls()"
            ))
        })?;

        let feature_entries = feature_map_value
            .downcast_ref::<FeatureMap>()
            .expect("feature_map_value is a FeatureMap")
            .entries();

        for (_, type_value, instance_value) in feature_entries {
            let enabled = instance_value
                .downcast_ref::<FeatureInstance>()
                .map(|i| i.enabled.get())
                .unwrap_or(true);

            if !enabled {
                continue;
            }

            if let Some(impl_fn) = extract_feature_impl_fn(type_value) {
                let type_id = extract_feature_type_id(type_value).unwrap_or(0);
                let cli_args = args_builder(type_id, heap);
                let merged = populate_feature_custom_args(type_value, instance_value, cli_args);
                let attrs_value = heap.alloc(merged);
                let fctx = heap.alloc(FeatureContext::new(attrs_value, trait_map_value));
                let store = self.loader.new_store(self.loader.repo_root.clone());
                let mut eval = Evaluator::new(&self.env.0);
                eval.set_loader(self.loader);
                eval.extra = Some(&store);
                eval.eval_function(impl_fn, &[fctx], &[]).map_err(|e| {
                    EvalError::UnknownError(anyhow!("Feature implementation failed: {:?}", e))
                })?;
            }
        }

        Ok(())
    }

    /// Phase 5: execute the selected task with pre-built args.
    ///
    /// The `TaskContext` is allocated on the shared heap. The task implementation
    /// is called via `eval_function` on a fresh evaluator over the shared module.
    /// Phase 5: execute the selected task with pre-built args.
    ///
    /// `args_builder` returns a pair of `Args`:
    /// - `all_args`: all CLI arg values, including Clap defaults.
    /// - `explicit_args`: only values the user explicitly provided on the command line.
    ///
    /// Precedence (highest to lowest):
    ///   1. Explicit CLI flag (`--count 5`)
    ///   2. `config.axl` override (`t.args.count = 2`)
    ///   3. Task definition default (`args.int(default = 1)`)
    pub fn execute_with_args(
        &mut self,
        task: Value<'v>,
        task_key: String,
        task_id: Option<String>,
        args_builder: impl FnOnce(Heap<'v>) -> (CliArgs<'v>, CliArgs<'v>),
    ) -> Result<Option<u8>, EvalError> {
        let ct = task
            .downcast_ref::<ConfiguredTask>()
            .ok_or_else(|| EvalError::UnknownError(anyhow!("task is not a ConfiguredTask")))?;

        let heap = self.heap();

        let task_impl = ct
            .implementation()
            .ok_or_else(|| EvalError::UnknownError(anyhow!("task has no implementation")))?;
        // The frozen heap is already kept alive by AxlLoader::loaded_modules (Phase 1 tasks)
        // or by the live heap's registration (ctx.tasks.add tasks), so to_value() is safe.
        let task_impl_fv = task_impl.to_value();

        let task_id = task_id.unwrap_or_else(|| Uuid::new_v4().to_string());

        // Build args with proper precedence:
        // 1. Start with config-only attr defaults from the task definition.
        // 2. Apply config_overrides (from config.axl) for keys the user did NOT set on the CLI.
        // 3. CLI args (with their own defaults) are already in all_args and win implicitly.
        let (mut task_args, explicit_args) = args_builder(heap);

        // Seed config-only arg defaults. These never go through Clap so they must be
        // injected here. We only insert if not already present (config_overrides below
        // may overwrite, and explicit CLI flags from args_builder already win).
        // Custom args with no storable default (e.g. lambda defaults) are seeded as None
        // so that `ctx.args.name` is always accessible.
        if let Some(frozen_task) = ct.task_def.downcast_ref::<FrozenTask>() {
            for (name, arg) in frozen_task.args().iter() {
                if let crate::engine::arg::Arg::Custom { default, .. } = arg {
                    if !task_args.contains_key(name.as_str()) {
                        let value = default
                            .map(|fv| fv.to_value())
                            .unwrap_or_else(|| heap.alloc(starlark::values::none::NoneType));
                        task_args.insert(name.clone(), value);
                    }
                }
            }
        }

        // Apply config.axl overrides, skipping keys the user explicitly set on the CLI.
        for (k, owned) in ct.config_overrides.borrow().iter() {
            if !explicit_args.contains_key(k) {
                if let Some(fv) = owned.value().unpack_frozen() {
                    task_args.insert(k.clone(), fv.to_value());
                }
            }
        }
        let task_info = TaskInfo {
            name: ct.get_name(),
            group: ct.get_group(),
            task_key: task_key.clone(),
            task_id: task_id.clone(),
        };

        // Build task-scoped TraitMap from the global trait map
        let task_trait_map = match self.trait_map_value {
            Some(tmap_val) => {
                if let Some(tmap) = tmap_val.downcast_ref::<TraitMap>() {
                    tmap.scoped(&ct.trait_type_ids, heap)
                } else {
                    heap.alloc(TraitMap::new())
                }
            }
            None => heap.alloc(TraitMap::new()),
        };

        let startup_flags = heap.alloc(AllocList([] as [String; 0]));
        let bazel = heap.alloc(Bazel { startup_flags });
        let context = heap.alloc(TaskContext::new(
            task_args,
            task_trait_map,
            task_info,
            bazel,
        ));

        let store = self.loader.new_store(ct.path.clone());
        let mut eval = Evaluator::new(&self.env.0);
        eval.set_loader(self.loader);
        eval.extra = Some(&store);

        let ret = eval.eval_function(task_impl_fv, &[context], &[])?;
        Ok(ret.unpack_i32().map(|ex| ex as u8))
    }

    pub fn finish(self) -> FinishedEval {
        FinishedEval
    }
}

pub struct FinishedEval;
