use starlark::environment::Module;
use starlark::eval::Evaluator;
use starlark::syntax::{AstModule, Dialect};
use starlark::values;
use std::fs;
use std::path::Path;

use crate::engine::config_context::{ConfigContext, TaskMut};
use crate::engine::store::AxlStore;
use crate::eval::load::{AxlLoader, ModuleScope};

use super::error::EvalError;

/// Evaluator for running config.axl files.a
#[derive(Debug)]
pub struct ConfigEvaluator<'l, 'p> {
    loader: &'l AxlLoader<'p>,
    store: AxlStore,
    module: Module,
}

impl<'l, 'p> ConfigEvaluator<'l, 'p> {
    /// Creates a new AxlScriptEvaluator with the given module root.
    pub fn new(loader: &'l AxlLoader<'p>) -> Self {
        Self {
            loader,
            store: AxlStore::new(),
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
    ) -> Result<(), EvalError> {
        self.loader.module_stack.borrow_mut().push(scope);

        let mut eval = Evaluator::new(&self.module);
        eval.set_loader(self.loader);
        eval.extra = Some(&self.store);

        let heap = self.module.heap();
        let context = heap.alloc(ConfigContext::new(tasks, heap));

        for path in paths {
            assert!(path.is_absolute());

            // Push the script path onto the LOAD_STACK (used to detect circular loads)
            self.loader.load_stack.borrow_mut().push(path.to_path_buf());

            // Load and evaluate the script
            let raw = fs::read_to_string(&path)?;
            let ast = AstModule::parse(&path.to_string_lossy(), raw, &self.loader.dialect)?;

            eval.eval_module(ast, &self.loader.globals)?;

            let def = self.module.get("config").ok_or(EvalError::MissingSymbol(
                path.to_path_buf(),
                "config".into(),
            ))?;
            eval.eval_function(def, &[context], &[])?;

            // Pop the script path off of the LOAD_STACK
            self.loader.load_stack.borrow_mut().pop();
        }

        drop(eval);
        Ok(())
    }
}
