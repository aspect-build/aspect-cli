use std::path::{Path, PathBuf};
use std::rc::Rc;

use anyhow::anyhow;
use starlark::environment::{FrozenModule, Module};
use starlark::eval::Evaluator;
use starlark::values::{self, Heap, ValueLike};

use crate::engine::store::AxlStore;
use crate::engine::task::Task;
use crate::engine::task::{AsTaskLike, FrozenTask, TaskLike};
use crate::engine::task_args::TaskArgs;
use crate::engine::task_context::TaskContext;

use super::error::EvalError;
use super::load::{AxlLoader, ModuleScope};
use super::load_path::join_confined;

/// Represents the result of evaluating an .axl script, encapsulating the module,
/// path, and store for task definition retrieval and execution.
#[derive(Debug, Clone)]
pub struct EvaluatedAxlScript {
    // Relative path of the script inside ModuleScope
    pub path: PathBuf,
    pub scope: ModuleScope,
    module: Rc<Module>,
    store: AxlStore,
}

impl EvaluatedAxlScript {
    fn new(scope: ModuleScope, path: PathBuf, store: AxlStore, module: Module) -> Self {
        Self {
            module: Rc::new(module),
            scope,
            path,
            store,
        }
    }

    /// Retrieves a task definition from the evaluated module by symbol name.
    pub fn task_definition(&self, symbol: &str) -> Result<&dyn TaskLike, EvalError> {
        let def = self.module.get(symbol).ok_or(EvalError::MissingSymbol(
            self.path.clone(),
            symbol.to_string(),
        ))?;
        if let Some(task) = def.downcast_ref::<Task>() {
            return Ok(task.as_task());
        } else if let Some(task) = def.downcast_ref::<FrozenTask>() {
            return Ok(task.as_task());
        } else {
            return Err(EvalError::UnknownError(anyhow!("expected type of Task")));
        }
    }

    pub fn names(&self) -> Vec<&str> {
        self.module
            .names()
            .filter(|symbol| self.has_task(symbol))
            .map(|sym| sym.as_str())
            .collect()
    }

    pub fn has_task(&self, symbol: &str) -> bool {
        let val = self.module.get(symbol);
        if let Some(val) = val {
            if val.downcast_ref::<Task>().is_none() && val.downcast_ref::<FrozenTask>().is_none() {
                return false;
            }
            return true;
        }
        false
    }

    pub fn has_name(&self, symbol: &str) -> bool {
        self.module.get(symbol).is_some()
    }

    pub fn get_variable<'v>(&'v self, symbol: &str) -> Option<values::Value<'v>> {
        self.module.get(symbol)
    }

    /// Executes a task from the module by symbol, providing arguments and returning the exit code.
    pub fn execute_task(
        &self,
        symbol: &str,
        args: impl FnOnce(&Heap) -> TaskArgs,
    ) -> Result<Option<u8>, EvalError> {
        let def = self.module.get(symbol).ok_or(EvalError::MissingSymbol(
            self.path.clone(),
            symbol.to_string(),
        ))?;

        let heap = self.module.heap();
        let args = args(heap);
        let context = heap.alloc(TaskContext::new(args));
        let mut eval = Evaluator::new(&self.module);
        eval.extra = Some(&self.store);
        let ret = if let Some(val) = def.downcast_ref::<Task>() {
            eval.eval_function(val.implementation(), &[context], &[])?
        } else if let Some(val) = def.downcast_ref::<FrozenTask>() {
            eval.eval_function(val.implementation().to_value(), &[context], &[])?
        } else {
            return Err(EvalError::UnknownError(anyhow::anyhow!(
                "expected value of type Task"
            )));
        };
        Ok(ret.unpack_i32().map(|ex| ex as u8))
    }
}

/// The core evaluator for .axl files, holding configuration like module root,
/// Starlark dialect, globals, and store. Used to evaluate .axl files securely.
#[derive(Debug)]
pub struct TaskEvaluator<'l, 'p> {
    loader: &'l AxlLoader<'p>,
}

impl<'l, 'p> TaskEvaluator<'l, 'p> {
    /// Creates a new AxlScriptEvaluator with the given module root.
    pub fn new(loader: &'l AxlLoader<'p>) -> Self {
        Self { loader }
    }

    /// Evaluates the given .axl script path relative to the module root, returning
    /// the evaluated script or an error. Performs security checks to ensure the script
    /// file is within the module root.
    pub fn eval(&self, scope: ModuleScope, path: &Path) -> Result<EvaluatedAxlScript, EvalError> {
        assert!(path.is_relative());

        let abs_path = join_confined(&scope.path, path)?;
        self.loader.module_stack.borrow_mut().push(scope);
        let module = self.loader.eval_module(&abs_path)?;
        let scope = self
            .loader
            .module_stack
            .borrow_mut()
            .pop()
            .expect("just pushed a scope");

        // Return the evaluated script
        Ok(EvaluatedAxlScript::new(
            scope,
            path.to_path_buf(),
            self.loader.store.clone(),
            module,
        ))
    }
}
