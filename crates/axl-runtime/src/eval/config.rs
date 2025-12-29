use starlark::environment::Module;
use starlark::eval::Evaluator;
use starlark::syntax::AstModule;
use starlark::values::ValueLike;
use std::fs;
use std::path::Path;

use crate::engine::config::{ConfigContext, TaskMut};
use crate::eval::load::{AxlLoader, ModuleScope};

use super::error::EvalError;

/// Evaluator for running config.axl files.a
#[derive(Debug)]
pub struct ConfigEvaluator<'l, 'p> {
    loader: &'l AxlLoader<'p>,
    module: Module,
}

impl<'l, 'p> ConfigEvaluator<'l, 'p> {
    /// Creates a new AxlScriptEvaluator with the given module root.
    pub fn new(loader: &'l AxlLoader<'p>) -> Self {
        Self {
            loader,
            module: Module::new(),
        }
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
        self.loader.module_stack.borrow_mut().push(scope);

        let heap = self.module.heap();
        let context = heap.alloc(ConfigContext::new(tasks, heap));

        for path in paths {
            assert!(path.is_absolute());

            // Push the script path onto the LOAD_STACK (used to detect circular loads)
            self.loader.load_stack.borrow_mut().push(path.to_path_buf());

            // Load and evaluate the script
            let raw = fs::read_to_string(&path)?;
            let ast = AstModule::parse(&path.to_string_lossy(), raw, &self.loader.dialect)?;

            let store = self.loader.store(path.to_path_buf());
            let mut eval = Evaluator::new(&self.module);
            eval.set_loader(self.loader);
            eval.extra = Some(&store);
            eval.eval_module(ast, &self.loader.globals)?;
            let def = self
                .module
                .get("config")
                .ok_or(EvalError::MissingSymbol("config".into()))?;
            eval.eval_function(def, &[context], &[])?;
            drop(eval);
            drop(store);

            // Pop the script path off of the LOAD_STACK
            self.loader.load_stack.borrow_mut().pop();
        }
        let ctx = context.downcast_ref::<ConfigContext>().unwrap();
        Ok(ctx)
    }
}
