use anyhow::anyhow;
use starlark::environment::Module;
use starlark::eval::Evaluator;
use starlark::values::{Heap, Value, ValueLike};
use std::path::Path;

use crate::engine::config::{ConfigContext, TaskMut};
use crate::eval::load::{AxlLoader, ModuleScope};
use crate::eval::load_path::join_confined;

use super::error::EvalError;

/// Evaluator for running config.axl files.a
#[derive(Debug)]
pub struct ConfigEvaluator<'l, 'p> {
    loader: &'l AxlLoader<'p>,
}

impl<'l, 'p> ConfigEvaluator<'l, 'p> {
    /// Creates a new AxlScriptEvaluator with the given module root.
    pub fn new(loader: &'l AxlLoader<'p>) -> Self {
        Self { loader }
    }

    /// Evaluates the given .axl script path relative to the module root, returning
    /// the evaluated script or an error. Performs security checks to ensure the script
    /// file is within the module root.
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

        // Return the evaluated script
        Ok(module)
    }

    /// Evaluates the given .axl script path relative to the module root, returning
    /// the evaluated script or an error. Performs security checks to ensure the script
    /// file is within the module root.
    pub fn run_all<'v>(
        &'v self,
        scope: ModuleScope,
        paths: Vec<&Path>,
        tasks: Vec<TaskMut<'v>>,
    ) -> Result<&'v ConfigContext<'v>, EvalError> {
        self.loader.module_stack.borrow_mut().push(scope.clone());

        let eval_module = Box::leak(Box::new(Module::new()));
        let context_module = Box::leak(Box::new(Module::new()));
        let heap: &'v Heap =
            unsafe { std::mem::transmute::<&Heap, &'v Heap>(context_module.heap()) };
        let context = heap.alloc(ConfigContext::new(tasks, heap));
        let ctx = context.downcast_ref::<ConfigContext<'v>>().unwrap();

        for path in paths {
            assert!(path.is_absolute());

            let rel_path = path
                .strip_prefix(&scope.path)
                .map_err(|e| EvalError::UnknownError(anyhow!("Failed to strip prefix: {e}")))?
                .to_path_buf();

            let config_module = self.eval(scope.clone(), &rel_path)?;

            let frozen = config_module
                .freeze()
                .map_err(|e| EvalError::UnknownError(anyhow!(e)))?;

            let def = frozen
                .get("config")
                .map_err(|_| EvalError::MissingSymbol("config".into()))?;

            let func = def.value();

            // Adjust the lifetime of func to 'v so it can be used within eval.eval_function below.
            // This is necessary because the type system prevents mixing Values from different heaps
            // without it, but in this case for the lifetime of frozen function called for side effects
            // on a shared context, it is safe in practice.
            let func = unsafe { std::mem::transmute::<Value, Value<'v>>(func) };

            let store = self.loader.new_store(path.to_path_buf());
            let mut eval = Evaluator::new(eval_module);
            eval.set_loader(self.loader);
            eval.extra = Some(&store);
            eval.eval_function(func, &[context], &[])?;
            drop(eval);
            drop(store);

            ctx.add_config_module(frozen);
        }

        // Set and freeze initial configs if not set
        let mut to_set: Vec<(&'v TaskMut<'v>, Value<'v>)> = Vec::new();
        for task in ctx.tasks() {
            let config = *task.config.borrow();
            if config.is_none() {
                let initial = task.initial_config();
                to_set.push((task, initial));
            }
        }
        for (task, initial) in to_set {
            let temp_module = Module::new();
            let short_initial: Value = unsafe { std::mem::transmute(initial) };
            temp_module.set("temp", short_initial);
            let frozen = temp_module.freeze().expect("freeze failed");
            let frozen_val: Value<'v> =
                unsafe { std::mem::transmute(frozen.get("temp").expect("get").value()) };
            task.config.replace(frozen_val);
            *task.frozen_config_module.borrow_mut() = Some(frozen);
        }

        self.loader.module_stack.borrow_mut().pop();
        Ok(ctx)
    }
}
