use anyhow::anyhow;
use starlark::environment::{FrozenModule, Module};
use starlark::eval::Evaluator;
use starlark::values::{Value, ValueLike};
use starlark_map::small_map::SmallMap;
use std::path::Path;
use std::path::PathBuf;

use crate::engine::config::feature_context::FeatureContext;
use crate::engine::config::feature_map::{FrozenFeatureMap, construct_features};
use crate::engine::config::fragment_map::{FrozenFragmentMap, construct_fragments};
use crate::engine::config::{ConfigContext, ConfiguredTask};
use crate::engine::types::feature::{FeatureInstance, extract_feature_impl_fn};
use crate::engine::types::fragment::extract_fragment_type_id;
use crate::eval::load::{AxlLoader, ModuleScope};
use crate::eval::load_path::join_confined;

use super::error::EvalError;

/// Result of running all config evaluations.
pub struct ConfigResult {
    /// The configured tasks.
    pub tasks: Vec<ConfiguredTask>,
    /// Fragment type IDs mapped to their (type_value, instance_value) pairs.
    pub fragment_data: Vec<(u64, Value<'static>, Value<'static>)>,
    /// Feature type IDs mapped to their (type_value, instance_value) pairs.
    pub feature_data: Vec<(u64, Value<'static>, Value<'static>)>,
    /// Keeps the context module's frozen heap alive so fragment/feature data values remain valid.
    _context_module: FrozenModule,
}

/// Evaluator for running config.axl files.
#[derive(Debug)]
pub struct ConfigEvaluator<'l, 'p> {
    loader: &'l AxlLoader<'p>,
}

impl<'l, 'p> ConfigEvaluator<'l, 'p> {
    /// Creates a new ConfigEvaluator with the given loader.
    pub fn new(loader: &'l AxlLoader<'p>) -> Self {
        Self { loader }
    }

    /// Evaluates the given .axl script path relative to the module root.
    pub fn eval(&self, scope: ModuleScope, path: &Path) -> Result<FrozenModule, EvalError> {
        assert!(path.is_relative());

        let abs_path = join_confined(&scope.path, path)?;

        // push the current scope to stack
        self.loader.module_stack.borrow_mut().push(scope);
        let frozen = self.loader.eval_module(&abs_path)?;
        // pop the current
        let _scope = self
            .loader
            .module_stack
            .borrow_mut()
            .pop()
            .expect("just pushed a scope");

        Ok(frozen)
    }

    /// Evaluates all config files with the given tasks and feature types.
    ///
    /// Execution order:
    /// 1. Collect fragment types from all tasks
    /// 2. Auto-construct default fragment instances
    /// 3. Auto-construct default feature instances (enabled=True)
    /// 4. Create ConfigContext with both maps
    /// 5. Evaluate each config file (user mutates ctx.fragments[X] and ctx.features[Y])
    /// 6. Run each enabled feature's implementation(FeatureContext) to inject into fragments
    /// 7. Freeze
    pub fn run_all(
        &self,
        scoped_configs: Vec<(ModuleScope, PathBuf, String)>,
        tasks: Vec<ConfiguredTask>,
        feature_types: Vec<(u64, Value<'static>)>,
    ) -> Result<ConfigResult, EvalError> {
        let (result_tasks, fragment_map_key, feature_map_key, frozen_ctx_module) =
            Module::with_temp_heap(|context_module| {
                let heap = context_module.heap();

                // Collect fragment types from all tasks
                let mut fragment_types: SmallMap<u64, Value> = SmallMap::new();
                for task in &tasks {
                    let frozen_task = task
                        .as_frozen_task()
                        .expect("tasks should be frozen at this point");
                    for frag_fv in frozen_task.fragments() {
                        let frag_value = frag_fv.to_value();
                        if let Some(id) = extract_fragment_type_id(frag_value) {
                            fragment_types.entry(id).or_insert(frag_value);
                        }
                    }
                }

                // Auto-construct default fragment instances
                let fragment_pairs: Vec<(u64, Value)> =
                    fragment_types.into_iter().map(|(id, v)| (id, v)).collect();

                let fragment_map = {
                    let store = self.loader.new_store(self.loader.repo_root.clone());
                    let mut eval = Evaluator::new(&context_module);
                    eval.set_loader(self.loader);
                    eval.extra = Some(&store);
                    construct_fragments(&fragment_pairs, &mut eval, heap)?
                };

                let fragment_map_value = heap.alloc(fragment_map);

                // Auto-construct default feature instances
                // SAFETY: feature_types live on frozen heaps that outlive this closure
                let feature_pairs: Vec<(u64, Value)> = feature_types
                    .iter()
                    .map(|(id, fv)| {
                        (*id, unsafe {
                            std::mem::transmute::<Value<'static>, Value>(*fv)
                        })
                    })
                    .collect();

                let feature_map = {
                    let store = self.loader.new_store(self.loader.repo_root.clone());
                    let mut eval = Evaluator::new(&context_module);
                    eval.set_loader(self.loader);
                    eval.extra = Some(&store);
                    construct_features(&feature_pairs, &mut eval)?
                };

                let feature_map_value = heap.alloc(feature_map);

                // Create ConfigContext with tasks, fragment map, and feature map
                let context_value = heap.alloc(ConfigContext::new(
                    tasks,
                    fragment_map_value,
                    feature_map_value,
                    heap,
                ));
                let ctx = context_value
                    .downcast_ref::<ConfigContext>()
                    .expect("just allocated ConfigContext");

                // Evaluate each config file
                for (scope, path, function_name) in &scoped_configs {
                    self.loader.module_stack.borrow_mut().push(scope.clone());

                    let rel_path = path
                        .strip_prefix(&scope.path)
                        .map_err(|e| {
                            EvalError::UnknownError(anyhow!("Failed to strip prefix: {e}"))
                        })?
                        .to_path_buf();

                    let frozen = self.eval(scope.clone(), &rel_path)?;

                    let def = frozen
                        .get(function_name)
                        .map_err(|_| EvalError::MissingSymbol(function_name.clone()))?;

                    let func = def.value();
                    let func = unsafe { std::mem::transmute::<Value, Value>(func) };

                    let store = self.loader.new_store(path.to_path_buf());
                    {
                        let mut eval = Evaluator::new(&context_module);
                        eval.set_loader(self.loader);
                        eval.extra = Some(&store);
                        eval.eval_function(func, &[context_value], &[])?;
                    }

                    ctx.add_config_module(frozen);
                    self.loader.module_stack.borrow_mut().pop();
                }

                // Run each enabled feature's implementation function.
                // At this point config.axl has fully run, so feature attrs are final.
                {
                    let store = self.loader.new_store(self.loader.repo_root.clone());
                    let mut eval = Evaluator::new(&context_module);
                    eval.set_loader(self.loader);
                    eval.extra = Some(&store);

                    // Collect entries first to avoid borrow conflict
                    let feature_entries = ctx
                        .feature_map_value()
                        .downcast_ref::<crate::engine::config::feature_map::FeatureMap>()
                        .expect("feature_map_value is a FeatureMap")
                        .entries();

                    for (_, type_value, instance_value) in feature_entries {
                        // Skip disabled features
                        let enabled = instance_value
                            .downcast_ref::<FeatureInstance>()
                            .map(|i| i.enabled.get())
                            .unwrap_or(true);

                        if !enabled {
                            continue;
                        }

                        if let Some(impl_fn) = extract_feature_impl_fn(type_value) {
                            let fctx =
                                heap.alloc(FeatureContext::new(instance_value, fragment_map_value));
                            eval.eval_function(impl_fn, &[fctx], &[]).map_err(|e| {
                                EvalError::UnknownError(anyhow!(
                                    "Feature implementation failed for {}: {:?}",
                                    type_value,
                                    e
                                ))
                            })?;
                        }
                    }
                }

                // Clone tasks from the context to return
                let result_tasks: Vec<ConfiguredTask> =
                    ctx.tasks().iter().map(|t| (*t).clone()).collect();

                // Store context, fragment map, and feature map so they survive freezing
                context_module.set("__ctx__", context_value);
                context_module.set("__fmap__", fragment_map_value);
                context_module.set("__featmap__", feature_map_value);

                let frozen_ctx_module = context_module
                    .freeze()
                    .map_err(|e| EvalError::UnknownError(anyhow!("{:?}", e)))?;

                Ok::<_, EvalError>((
                    result_tasks,
                    "__fmap__".to_string(),
                    "__featmap__".to_string(),
                    frozen_ctx_module,
                ))
            })?;

        // Extract fragment data from the frozen module's FragmentMap
        let fmap_owned = frozen_ctx_module
            .get(&fragment_map_key)
            .map_err(|e| EvalError::UnknownError(anyhow!("{:?}", e)))?;
        let fmap = fmap_owned
            .value()
            .downcast_ref::<FrozenFragmentMap>()
            .expect("stored FragmentMap");
        let fragment_data: Vec<(u64, Value<'static>, Value<'static>)> = fmap
            .entries()
            .into_iter()
            .map(|(id, tv, iv)| {
                // SAFETY: These values live on frozen_ctx_module's frozen heap
                let tv: Value<'static> = unsafe { std::mem::transmute(tv) };
                let iv: Value<'static> = unsafe { std::mem::transmute(iv) };
                (id, tv, iv)
            })
            .collect();

        // Extract feature data from the frozen module's FeatureMap
        let featmap_owned = frozen_ctx_module
            .get(&feature_map_key)
            .map_err(|e| EvalError::UnknownError(anyhow!("{:?}", e)))?;
        let featmap = featmap_owned
            .value()
            .downcast_ref::<FrozenFeatureMap>()
            .expect("stored FeatureMap");
        let feature_data: Vec<(u64, Value<'static>, Value<'static>)> = featmap
            .entries()
            .into_iter()
            .map(|(id, tv, iv)| {
                let tv: Value<'static> = unsafe { std::mem::transmute(tv) };
                let iv: Value<'static> = unsafe { std::mem::transmute(iv) };
                (id, tv, iv)
            })
            .collect();

        Ok(ConfigResult {
            tasks: result_tasks,
            fragment_data,
            feature_data,
            _context_module: frozen_ctx_module,
        })
    }
}
