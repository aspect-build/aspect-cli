use anyhow::anyhow;
use starlark::environment::Module;
use starlark::eval::Evaluator;
use starlark::values::{Value, ValueLike};
use starlark_map::small_map::SmallMap;
use std::path::Path;
use std::path::PathBuf;

use crate::engine::config::fragment_map::{FragmentMap, construct_fragments};
use crate::engine::config::{ConfigContext, ConfiguredTask};
use crate::engine::types::fragment::extract_fragment_type_id;
use crate::eval::load::{AxlLoader, ModuleScope};
use crate::eval::load_path::join_confined;

use super::error::EvalError;

/// Result of running all config evaluations.
pub struct ConfigResult {
    /// The configured tasks.
    pub tasks: Vec<ConfiguredTask>,
    /// Fragment type IDs mapped to their (type_value, instance_value) pairs.
    /// These are the globally-configured fragment instances that tasks will use.
    pub fragment_data: Vec<(u64, Value<'static>, Value<'static>)>,
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
    pub fn eval(&self, scope: ModuleScope, path: &Path) -> Result<Module, EvalError> {
        assert!(path.is_relative());

        let abs_path = join_confined(&scope.path, path)?;

        // push the current scope to stack
        self.loader.module_stack.borrow_mut().push(scope);
        let module = self.loader.eval_module(&abs_path)?;
        // pop the current
        let _scope = self
            .loader
            .module_stack
            .borrow_mut()
            .pop()
            .expect("just pushed a scope");

        Ok(module)
    }

    /// Evaluates all config files with the given tasks.
    ///
    /// This method:
    /// 1. Collects fragment types from all tasks
    /// 2. Auto-constructs default fragment instances
    /// 3. Creates a FragmentMap and ConfigContext
    /// 4. Evaluates each config file, calling its `config` function
    /// 5. Returns the modified tasks and fragment data
    pub fn run_all(
        &self,
        scoped_configs: Vec<(ModuleScope, PathBuf, String)>,
        tasks: Vec<ConfiguredTask>,
    ) -> Result<ConfigResult, EvalError> {
        // Create temporary modules for evaluation
        let eval_module = Box::leak(Box::new(Module::new()));
        let context_module = Box::leak(Box::new(Module::new()));

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
            let mut eval = Evaluator::new(eval_module);
            eval.set_loader(self.loader);
            construct_fragments(&fragment_pairs, &mut eval, heap)?
        };

        let fragment_map_value = heap.alloc(fragment_map);

        // Create ConfigContext with tasks and fragment map
        let context_value = heap.alloc(ConfigContext::new(tasks, fragment_map_value, heap));
        let ctx = context_value
            .downcast_ref::<ConfigContext>()
            .expect("just allocated ConfigContext");

        // Evaluate each config file with its associated scope
        for (scope, path, function_name) in &scoped_configs {
            self.loader.module_stack.borrow_mut().push(scope.clone());

            let rel_path = path
                .strip_prefix(&scope.path)
                .map_err(|e| EvalError::UnknownError(anyhow!("Failed to strip prefix: {e}")))?
                .to_path_buf();

            // Evaluate the config module
            let config_module = self.eval(scope.clone(), &rel_path)?;

            // Freeze the config module to get the config function
            let frozen = config_module
                .freeze()
                .map_err(|e| EvalError::UnknownError(anyhow!("{:?}", e)))?;

            // Get the config function
            let def = frozen
                .get(function_name)
                .map_err(|_| EvalError::MissingSymbol(function_name.clone()))?;

            let func = def.value();

            // SAFETY: The frozen config function is called for side effects on the context.
            // The context lives as long as this function, which outlives the eval call.
            let func = unsafe { std::mem::transmute::<Value, Value>(func) };

            // Create evaluator and run the config function
            let store = self.loader.new_store(path.to_path_buf());
            let mut eval = Evaluator::new(eval_module);
            eval.set_loader(self.loader);
            eval.extra = Some(&store);
            eval.eval_function(func, &[context_value], &[])?;
            drop(eval);
            drop(store);

            // Keep the frozen module alive for the duration
            ctx.add_config_module(frozen);

            self.loader.module_stack.borrow_mut().pop();
        }

        // Clone tasks from the context to return
        let result_tasks: Vec<ConfiguredTask> = ctx.tasks().iter().map(|t| (*t).clone()).collect();

        // Extract fragment data from the FragmentMap
        let fmap = fragment_map_value
            .downcast_ref::<FragmentMap>()
            .expect("just allocated FragmentMap");
        let fragment_data: Vec<(u64, Value<'static>, Value<'static>)> = fmap
            .entries()
            .into_iter()
            .map(|(id, tv, iv)| {
                // SAFETY: These values live on context_module's leaked heap
                let tv: Value<'static> = unsafe { std::mem::transmute(tv) };
                let iv: Value<'static> = unsafe { std::mem::transmute(iv) };
                (id, tv, iv)
            })
            .collect();

        Ok(ConfigResult {
            tasks: result_tasks,
            fragment_data,
        })
    }
}
