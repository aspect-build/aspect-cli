use std::path::{Path, PathBuf};

use anyhow::anyhow;
use starlark::environment::{FrozenModule, Module};
use starlark::eval::Evaluator;
use starlark::values::list::AllocList;
use starlark::values::{Heap, Value, ValueLike};
use uuid::Uuid;

use crate::engine::arguments::Arguments;
use crate::engine::bazel::Bazel;
use crate::engine::config_context::ConfigContext;
use crate::engine::feature::{Feature, FeatureLike, FrozenFeature};
use crate::engine::feature_context::FeatureContext;
use crate::engine::feature_map::FeatureMap;
use crate::engine::task::{FrozenTask, Task, TaskLike};
use crate::engine::task_context::TaskContext;
use crate::engine::task_info::TaskInfo;
use crate::engine::task_map::TaskMap;
use crate::engine::telemetry::{self, ExporterSpec, Telemetry};
use crate::engine::r#trait::extract_trait_type_id;
use crate::engine::trait_map::TraitMap;
use crate::eval::error::EvalError;
use crate::eval::load::AxlLoader;
use crate::eval::task::FrozenTaskModuleLike;
use crate::module::Mod;

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

/// Multi-phase Starlark evaluator with one shared Module heap.
///
/// Phases:
/// 1. `eval`              — load task and feature scripts; inserts discovered
///                          `Task` and `Feature` values into `self.tasks` /
///                          `self.features` on the shared heap.
/// 2. `eval_config`       — run config.axl files; mutations visible via shared heap.
/// 3. `eval_feature_impls`— run enabled feature implementations.
/// 4. `execute_with_args` — call the selected task implementation.
pub struct MultiPhaseEval<'v, 'l> {
    env: &'l ModuleEnv<'v>,
    loader: &'l AxlLoader<'l>,
    /// Global task map on the shared heap. Eagerly allocated empty in `new()`;
    /// Phase 1 inserts each discovered task; `ctx.tasks.add(...)` may insert more
    /// during Phase 2.
    tasks: Value<'v>,
    /// Global feature map on the shared heap. Eagerly allocated empty in `new()`;
    /// Phase 1 inserts each discovered feature.
    features: Value<'v>,
    /// Global trait map allocated on the shared heap during Phase 2.
    trait_map_value: Option<Value<'v>>,
    /// Telemetry handle allocated on the heap and shared between
    /// ConfigContext and FeatureContext as `ctx.telemetry`. The runtime
    /// drains exporter specs out of it (via `drain_exporters`) after phase 3.
    telemetry_value: Value<'v>,
}

impl<'v, 'l> MultiPhaseEval<'v, 'l> {
    pub fn new(env: &'l ModuleEnv<'v>, loader: &'l AxlLoader<'l>) -> Self {
        let heap = env.heap();
        let telemetry_value = Telemetry::alloc(heap);
        MultiPhaseEval {
            env,
            loader,
            tasks: heap.alloc(TaskMap::new()),
            features: heap.alloc(FeatureMap::new()),
            trait_map_value: None,
            telemetry_value,
        }
    }

    /// Drain exporter specs collected from `ctx.telemetry.exporters.add(...)`
    /// during phases 2-3. Intended to be called once, after
    /// `execute_features_with_args` completes and before phase 4 begins.
    pub fn drain_exporters(&self) -> Vec<ExporterSpec> {
        telemetry::drain_exporters(self.telemetry_value)
    }

    fn heap(&self) -> Heap<'v> {
        self.env.0.heap()
    }

    /// Load and evaluate an AXL file via AxlLoader (per-file frozen module).
    /// The loader manages caching, cycle detection, and the module-scope stack.
    #[tracing::instrument(level = "debug", skip_all, fields(path = %abs_path.display()))]
    fn eval_file(&self, scope: &'l Mod, abs_path: &Path) -> Result<FrozenModule, EvalError> {
        self.loader.eval_module(scope, abs_path)
    }

    /// Phase 1: evaluate task and feature scripts plus module specs.
    ///
    /// Each file is loaded into its own frozen module (via `AxlLoader::eval_module`).
    /// `FrozenTask` and `FrozenFeature` values discovered in those modules are
    /// inserted into `self.tasks` and `self.features` on the shared heap.
    #[tracing::instrument(skip_all)]
    pub fn eval(
        &mut self,
        scripts: &[PathBuf],
        root_mod: &'l Mod,
        modules: &'l Vec<Mod>,
    ) -> Result<(), EvalError> {
        let heap = self.heap();
        let task_map = self
            .tasks
            .downcast_ref::<TaskMap>()
            .expect("self.tasks is a TaskMap");
        let feature_map = self
            .features
            .downcast_ref::<FeatureMap>()
            .expect("self.features is a FeatureMap");

        // Evaluate auto-discovered AXL scripts (axl_sources in repo root)
        for path in scripts {
            let frozen = self.eval_file(&root_mod, path)?;
            for symbol in frozen.tasks() {
                let owned = frozen
                    .get(symbol.as_str())
                    .map_err(|_| EvalError::MissingSymbol(symbol.clone()))?;
                let frozen_value = owned
                    .value()
                    .unpack_frozen()
                    .expect("value from FrozenModule is always frozen");
                heap.access_owned_frozen_value(&owned);
                let live_task = Task::from_frozen(frozen_value, heap);
                task_map.insert(heap.alloc(live_task));
            }
        }

        // Evaluate module specs (use_task and use_feature entries from MODULE.aspect)
        for mode in modules.iter().chain(vec![root_mod]) {
            for (abs_path, (label, symbols)) in mode.tasks.iter() {
                let frozen = self.eval_file(&mode, &abs_path)?;
                for symbol in symbols {
                    if !frozen.has_name(symbol) {
                        return Err(EvalError::UnknownError(anyhow!(
                            "task symbol {:?} not found in @{} module use_task({:?})",
                            symbol,
                            mode.name,
                            label
                        )));
                    }
                    if !frozen.has_task(symbol) {
                        return Err(EvalError::UnknownError(anyhow!(
                            "invalid use_task({:?}, {:?}) in @{} module",
                            label,
                            symbol,
                            mode.name
                        )));
                    }
                    let owned = frozen
                        .get(symbol)
                        .map_err(|_| EvalError::MissingSymbol(symbol.clone()))?;
                    let frozen_value = owned
                        .value()
                        .unpack_frozen()
                        .expect("value from FrozenModule is always frozen");
                    heap.access_owned_frozen_value(&owned);
                    let live_task = Task::from_frozen(frozen_value, heap);
                    task_map.insert(heap.alloc(live_task));
                }
            }

            for (abs_path, symbol) in mode.features.iter() {
                let frozen = self.eval_file(&mode, &abs_path)?;
                let owned = frozen
                    .get(symbol.as_str())
                    .map_err(|_| EvalError::MissingSymbol(symbol.clone()))?;
                let frozen_value = owned.value().unpack_frozen().ok_or_else(|| {
                    EvalError::UnknownError(anyhow!(
                        "symbol {:?} in {:?} did not freeze",
                        symbol,
                        abs_path
                    ))
                })?;
                if frozen_value.downcast_ref::<FrozenFeature>().is_none() {
                    return Err(EvalError::UnknownError(anyhow!(
                        "symbol {:?} in {:?} is not a feature",
                        symbol,
                        abs_path
                    )));
                }
                // Register the frozen heap as a dependency of the live heap so the
                // FrozenValue back-pointer (kept on the thawed Feature) stays valid.
                heap.access_owned_frozen_value(&owned);
                let live_feature = Feature::from_frozen(frozen_value, heap);
                feature_map.insert(heap.alloc(live_feature));
            }
        }

        Ok(())
    }

    /// Snapshot of all tasks discovered in Phase 1 (plus any added during Phase 2 via
    /// `ctx.tasks.add(...)`), borrowed as `&dyn TaskLike` so callers don't have to
    /// downcast `Value`s themselves.
    pub fn tasks(&self) -> Vec<&'v dyn TaskLike<'v>> {
        self.tasks
            .downcast_ref::<TaskMap>()
            .expect("self.tasks is a TaskMap")
            .values()
            .into_iter()
            .map(|v| -> &'v dyn TaskLike<'v> {
                if let Some(t) = v.downcast_ref::<Task<'v>>() {
                    return t;
                }
                if let Some(t) = v.downcast_ref::<FrozenTask>() {
                    return t;
                }
                panic!("TaskMap entry is not a task: {}", v.get_type())
            })
            .collect()
    }

    /// Snapshot of all features discovered in Phase 1, borrowed as `&dyn FeatureLike`.
    pub fn features(&self) -> Vec<&'v dyn FeatureLike<'v>> {
        self.features
            .downcast_ref::<FeatureMap>()
            .expect("self.features is a FeatureMap")
            .values()
            .into_iter()
            .map(|v| -> &'v dyn FeatureLike<'v> {
                if let Some(f) = v.downcast_ref::<Feature<'v>>() {
                    return f;
                }
                if let Some(f) = v.downcast_ref::<FrozenFeature>() {
                    return f;
                }
                panic!("FeatureMap entry is not a feature: {}", v.get_type())
            })
            .collect()
    }

    /// Phase 2: evaluate config files against the task and feature values from Phase 1.
    ///
    /// A `TraitMap` is allocated on the shared heap. Config functions mutate `Task`
    /// and `Feature` values in-place via `set_attr` — the same `Value<'v>` objects
    /// discovered in Phase 1 are reused, so subsequent phases see the updated state.
    /// `ctx.tasks.add(...)` may insert additional tasks into `self.tasks`.
    #[tracing::instrument(name = "execute.configs", skip_all)]
    pub fn execute_configs(&mut self, configs: &[PathBuf], mode: &'l Mod) -> Result<(), EvalError> {
        let heap = self.heap();

        // Register trait types from all tasks into a TraitMap; instances are created lazily
        // on first access (from config.axl or the task implementation).
        let trait_map = TraitMap::new();
        for task_val in self
            .tasks
            .downcast_ref::<TaskMap>()
            .expect("self.tasks is a TaskMap")
            .values()
        {
            if let Some(task) = task_val.downcast_ref::<Task>() {
                for trait_value in task.traits() {
                    if let Some(id) = extract_trait_type_id(*trait_value) {
                        trait_map.insert(id, *trait_value);
                    }
                }
            }
        }
        let trait_map_value = heap.alloc(trait_map);
        self.trait_map_value = Some(trait_map_value);

        let context_value = heap.alloc(ConfigContext::new(
            self.tasks,
            self.trait_map_value.unwrap(),
            self.features,
            self.telemetry_value,
        ));

        for config_path in configs {
            let rel_path = config_path
                .strip_prefix(&mode.root)
                .map_err(|e| EvalError::UnknownError(anyhow!("Failed to strip prefix: {e}")))?
                .to_path_buf();

            let abs = mode.root.join(&rel_path);
            let frozen = self.eval_file(mode, &abs)?;

            let function_name = "config";
            let def = frozen
                .get(function_name)
                .map_err(|_| EvalError::MissingSymbol(function_name.to_string()))?;
            let func = heap.access_owned_frozen_value(&def);

            let mut eval = Evaluator::new(&self.env.0);
            eval.set_loader(self.loader);
            eval.extra = Some(&self.loader.env);
            eval.eval_function(func, &[context_value], &[])?;
        }

        Ok(())
    }

    /// Phase 3: run enabled feature implementations.
    ///
    /// Must be called after `eval_config` so that config files have had the opportunity
    /// to enable or disable features. `args_builder` builds the fully-merged `Arguments`
    /// for each feature (callers handle CLI/override/default precedence). Features whose
    /// resolved `enabled` is `False` are skipped.
    #[tracing::instrument(name = "execute.features", skip_all)]
    pub fn execute_features_with_args(
        &mut self,
        args_builder: impl Fn(&dyn FeatureLike<'v>, Heap<'v>) -> Arguments<'v>,
    ) -> Result<(), EvalError> {
        let heap = self.heap();

        let trait_map_value = self.trait_map_value.ok_or_else(|| {
            EvalError::UnknownError(anyhow!(
                "eval_config() must be called before execute_features_with_args()"
            ))
        })?;

        for feature in self.features() {
            let args = args_builder(feature, heap);

            let enabled = args
                .get("enabled")
                .and_then(|v| v.unpack_bool())
                .unwrap_or(true);
            if !enabled {
                continue;
            }

            let attrs_value = heap.alloc(args);
            let fctx = heap.alloc(FeatureContext::new(
                attrs_value,
                trait_map_value,
                self.telemetry_value,
            ));
            let mut eval = Evaluator::new(&self.env.0);
            eval.set_loader(self.loader);
            eval.extra = Some(&self.loader.env);
            eval.eval_function(feature.implementation(), &[fctx], &[])
                .map_err(|e| {
                    EvalError::UnknownError(anyhow!("Feature implementation failed: {:?}", e))
                })?;
        }

        Ok(())
    }

    /// Phase 4: execute the task at the given index with pre-built args.
    ///
    /// The index refers to the position in `self.tasks()` (same Vec returned to
    /// the CLI when building the command tree). `args_builder` returns the
    /// fully-merged `Arguments` (callers handle CLI/override/default precedence).
    #[tracing::instrument(
        name = "execute.task",
        skip_all,
        err,
        fields(
            task_key = %task_key,
            task = tracing::field::Empty,
            task_id = tracing::field::Empty,
            exit_code = tracing::field::Empty,
        )
    )]
    pub fn execute_tasks_with_args(
        &mut self,
        task_index: usize,
        task_key: String,
        task_id: Option<String>,
        args_builder: impl FnOnce(&dyn TaskLike<'v>, Heap<'v>) -> Arguments<'v>,
    ) -> Result<Option<u8>, EvalError> {
        let task = *self.tasks().get(task_index).ok_or_else(|| {
            EvalError::UnknownError(anyhow!("task index {} out of range", task_index))
        })?;

        let heap = self.heap();
        let task_id = task_id.unwrap_or_else(|| Uuid::new_v4().to_string());
        let task_args = args_builder(task, heap);

        let span = tracing::Span::current();
        span.record("task", task.name());
        span.record("task_id", task_id.as_str());

        let task_info = TaskInfo {
            name: task.name(),
            group: task.group().clone(),
            task_key,
            task_id,
        };

        // Universal "task starting" announcement. Fires for every task —
        // built-in or custom — so users can see in CI logs which task is
        // running and what key/id it was invoked under, without each
        // task's AXL impl having to remember to print this itself. Phase
        // boundaries inside the task body are still announced from AXL
        // via `lib/lifecycle.axl::announce`; this line just frames each
        // task invocation.
        //
        // On Buildkite the line is wrapped in a `--- :aspect:` section
        // marker so it groups its task's output under a collapsible
        // header rather than floating above the existing "Workflows
        // Runner Environment" / "Health Check" / "Running bazel …"
        // sections that BazelDefaults emits. Off Buildkite the marker
        // adds nothing (BK groups on `---`; other terminals just show a
        // plain line) so we keep the simpler `→` form.
        let on_buildkite = std::env::var_os("BUILDKITE").is_some();
        let prefix = if on_buildkite {
            "--- :aspect: "
        } else {
            "→ "
        };
        if task_info.task_key != task_info.name {
            println!(
                "{prefix}Running task `{}` (key: {})",
                task_info.name, task_info.task_key
            );
        } else {
            println!("{prefix}Running task `{}`", task_info.name);
        }

        let task_trait_map = match self.trait_map_value {
            Some(tmap_val) => {
                if let Some(tmap) = tmap_val.downcast_ref::<TraitMap>() {
                    tmap.scoped(&task.trait_type_ids(), heap)
                } else {
                    heap.alloc(TraitMap::new())
                }
            }
            None => heap.alloc(TraitMap::new()),
        };

        let startup_flags = heap.alloc(AllocList([] as [String; 0]));
        let bazel = heap.alloc(Bazel { startup_flags });
        let context = heap.alloc(TaskContext::new(
            heap.alloc(task_args),
            task_trait_map,
            heap.alloc(task_info),
            bazel,
        ));

        let mut eval = Evaluator::new(&self.env.0);
        eval.set_loader(self.loader);
        eval.extra = Some(&self.loader.env);

        let ret = eval.eval_function(task.implementation(), &[context], &[])?;
        let exit_code = ret.unpack_i32().map(|ex| ex as u8);

        tracing::Span::current().record("exit_code", exit_code.unwrap_or(0) as i64);
        if let Some(code) = exit_code
            && code != 0
        {
            tracing::error!(exit_code = code as i64, "task exited with non-zero status");
        }

        Ok(exit_code)
    }

    pub fn finish(self) -> FinishedEval {
        FinishedEval
    }
}

pub struct FinishedEval;
