use anyhow::anyhow;
use starlark::environment::Module;
use starlark::eval::Evaluator;
use starlark::values::{Heap, ValueLike};
use std::path::Path;

use crate::engine::config::TaskMut;
use crate::engine::store::AxlStore;
use crate::engine::task::Task;
use crate::engine::task::{AsTaskLike, FrozenTask, TaskLike};
use crate::engine::task_args::TaskArgs;
use crate::engine::task_context::TaskContext;

use super::error::EvalError;
use super::load::{AxlLoader, ModuleScope};
use super::load_path::join_confined;

pub trait TaskModuleLike {
    fn tasks(&self) -> Vec<&str>;
    fn has_task(&self, symbol: &str) -> bool;
    fn has_name(&self, symbol: &str) -> bool;
    /// Retrieves a task definition from the evaluated module by symbol name.
    fn get_task(&self, symbol: &str) -> Result<&dyn TaskLike, EvalError>;
    fn execute_task<'v>(
        &'v self,
        store: AxlStore,
        task: &TaskMut<'v>,
        args: impl FnOnce(&Heap) -> TaskArgs,
    ) -> Result<Option<u8>, EvalError>;
}

impl TaskModuleLike for Module {
    fn get_task(&self, symbol: &str) -> Result<&dyn TaskLike, EvalError> {
        let def = self
            .get(symbol)
            .ok_or(EvalError::MissingSymbol(symbol.to_string()))?;
        if let Some(task) = def.downcast_ref::<Task>() {
            return Ok(task.as_task());
        } else if let Some(task) = def.downcast_ref::<FrozenTask>() {
            return Ok(task.as_task());
        } else {
            return Err(EvalError::UnknownError(anyhow!("expected type of Task")));
        }
    }

    fn tasks(&self) -> Vec<&str> {
        self.names()
            .filter(|symbol| self.has_task(symbol))
            .map(|sym| sym.as_str())
            .collect()
    }

    fn has_task(&self, symbol: &str) -> bool {
        let val = self.get(symbol);
        if let Some(val) = val {
            if val.downcast_ref::<Task>().is_none() && val.downcast_ref::<FrozenTask>().is_none() {
                return false;
            }
            return true;
        }
        false
    }

    fn has_name(&self, symbol: &str) -> bool {
        self.get(symbol).is_some()
    }

    /// Executes a task from the module by symbol, providing arguments and returning the exit code.
    fn execute_task<'v>(
        &'v self,
        store: AxlStore,
        task: &TaskMut<'v>,
        args: impl FnOnce(&Heap) -> TaskArgs,
    ) -> Result<Option<u8>, EvalError> {
        let heap = self.heap();
        let args = args(heap);
        let context = heap.alloc(TaskContext::new(args));
        let mut eval = Evaluator::new(self);
        eval.extra = Some(&store);
        let ret = if let Some(val) = task.original.downcast_ref::<Task>() {
            eval.eval_function(val.implementation(), &[context], &[])?
        } else if let Some(val) = task.original.downcast_ref::<FrozenTask>() {
            eval.eval_function(val.implementation().to_value(), &[context], &[])?
        } else {
            return Err(EvalError::UnknownError(anyhow::anyhow!(
                "expected value of type Task"
            )));
        };
        drop(eval);
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
}
