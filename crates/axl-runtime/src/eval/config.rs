use anyhow::anyhow;
use starlark::environment::Module;
use starlark::eval::Evaluator;
use starlark::values::{Value, ValueLike};
use std::path::Path;
use std::path::PathBuf;

use crate::engine::config::{ConfigContext, ConfiguredTask};
use crate::eval::load::{AxlLoader, ModuleScope};
use crate::eval::load_path::join_confined;

use super::error::EvalError;

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
    /// 1. Creates a ConfigContext with the tasks
    /// 2. Evaluates config bindings for all tasks (lazy evaluation)
    /// 3. Evaluates each config file, calling its `config` function
    /// 4. Returns references to the modified tasks
    ///
    /// The tasks are modified in place via set_attr calls from config functions.
    pub fn run_all(
        &self,
        scoped_configs: Vec<(ModuleScope, PathBuf, String)>,
        tasks: Vec<ConfiguredTask>,
    ) -> Result<Vec<ConfiguredTask>, EvalError> {
        // Create temporary modules for evaluation
        let eval_module = Box::leak(Box::new(Module::new()));
        let context_module = Box::leak(Box::new(Module::new()));

        // Create ConfigContext with tasks
        let heap = context_module.heap();
        let context_value = heap.alloc(ConfigContext::new(tasks, heap));
        let ctx = context_value
            .downcast_ref::<ConfigContext>()
            .expect("just allocated ConfigContext");

        // Evaluate config bindings for all tasks (lazy evaluation)
        {
            let mut eval = Evaluator::new(eval_module);
            eval.set_loader(self.loader);
            for task_value in ctx.task_values() {
                let task = task_value
                    .downcast_ref::<ConfiguredTask>()
                    .expect("task_values should contain ConfiguredTask");
                task.evaluate_config(&mut eval)?;
            }
        }

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

        Ok(result_tasks)
    }
}
