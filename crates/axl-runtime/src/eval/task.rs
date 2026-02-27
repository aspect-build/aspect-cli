use anyhow::anyhow;
use starlark::environment::FrozenModule;
use starlark::environment::Module;
use starlark::eval::Evaluator;
use starlark::values::Heap;
use starlark::values::OwnedFrozenValue;
use starlark::values::ValueLike;
use std::collections::HashMap;
use std::path::Path;

use crate::engine::config::ConfiguredTask;
use crate::engine::store::AxlStore;
use crate::engine::task::FrozenTask;
use crate::engine::task_args::TaskArgs;
use crate::engine::task_context::TaskContext;
use crate::engine::task_info::TaskInfo;

use super::error::EvalError;
use super::load::{AxlLoader, ModuleScope};
use super::load_path::join_confined;

/// Trait for introspection operations on frozen modules.
pub trait FrozenTaskModuleLike {
    fn tasks(&self) -> Vec<String>;
    fn has_task(&self, symbol: &str) -> bool;
    fn has_name(&self, symbol: &str) -> bool;
    /// Retrieves a task definition from the frozen module by symbol name.
    fn get_task(&self, symbol: &str) -> Result<OwnedFrozenValue, EvalError>;
}

impl FrozenTaskModuleLike for FrozenModule {
    fn get_task(&self, symbol: &str) -> Result<OwnedFrozenValue, EvalError> {
        let def = self
            .get(symbol)
            .map_err(|e| EvalError::UnknownError(anyhow!(e)))?;
        // Verify it's actually a task
        let value = def.value();
        if value.downcast_ref::<FrozenTask>().is_none() {
            return Err(EvalError::UnknownError(anyhow!("expected type of Task")));
        }
        Ok(def)
    }

    fn tasks(&self) -> Vec<String> {
        self.names()
            .filter(|symbol| self.has_task(symbol.as_str()))
            .map(|sym| sym.as_str().to_string())
            .collect()
    }

    fn has_task(&self, symbol: &str) -> bool {
        if let Ok(val) = self.get(symbol) {
            if val.value().downcast_ref::<FrozenTask>().is_some() {
                return true;
            }
        }
        false
    }

    fn has_name(&self, symbol: &str) -> bool {
        self.get(symbol).is_ok()
    }
}

/// Executes a task using Buck2's temporary Module pattern.
///
/// This creates a temporary Module for the execution heap, allowing us to:
/// 1. Keep task implementations frozen (immutable, thread-safe)
/// 2. Allocate execution-time values on a temporary heap
/// 3. Drop the temporary heap after execution
///
/// The TaskContext is pre-frozen so WASM can access it directly via
/// `ctx.wasm` without needing runtime freezing.
pub fn execute_task(
    task: &ConfiguredTask,
    store: AxlStore,
    args: HashMap<String, String>,
) -> Result<Option<u8>, EvalError> {
    // Get config first - it needs to outlive the evaluator
    let config = task.get_config();
    let config_value = config.value();

    // Get the task implementation function
    let task_impl = task
        .implementation()
        .ok_or_else(|| EvalError::UnknownError(anyhow!("task has no implementation")))?;

    // Create a module for TaskContext and freeze it immediately
    // This allows WASM to access ctx directly without runtime freezing
    let ctx_module = Module::new();
    let heap = ctx_module.heap();
    let task_args = TaskArgs::from_map(args, heap);
    let task_info = TaskInfo {
        name: task.get_name(),
        group: task.get_group(),
    };
    let context = heap.alloc(TaskContext::new(task_args, config_value, task_info));
    ctx_module.set("__ctx__", context);

    let frozen_ctx_module = ctx_module
        .freeze()
        .map_err(|e| EvalError::UnknownError(anyhow!("{:?}", e)))?;
    // OwnedFrozenValue keeps the frozen heap alive for the duration of this function
    let frozen_context = frozen_ctx_module
        .get("__ctx__")
        .map_err(|e| EvalError::UnknownError(anyhow!("failed to get frozen context: {:?}", e)))?;

    // Create execution module for the evaluator
    let exec_module = Module::new();
    let mut eval = Evaluator::new(&exec_module);
    eval.extra = Some(&store);

    // Call frozen task implementation with frozen context
    let ret = eval.eval_function(task_impl.value(), &[frozen_context.value()], &[])?;

    Ok(ret.unpack_i32().map(|ex| ex as u8))
}

/// Executes a task with pre-built TaskArgs.
///
/// The TaskContext is pre-frozen so WASM can access it directly via
/// `ctx.wasm` without needing runtime freezing.
pub fn execute_task_with_args(
    task: &ConfiguredTask,
    store: AxlStore,
    args_builder: impl FnOnce(&Heap) -> TaskArgs,
) -> Result<Option<u8>, EvalError> {
    // Get config first - it needs to outlive the evaluator
    let config = task.get_config();
    let config_value = config.value();

    // Get the task implementation function
    let task_impl = task
        .implementation()
        .ok_or_else(|| EvalError::UnknownError(anyhow!("task has no implementation")))?;

    // Create a module for TaskContext and freeze it immediately
    // This allows WASM to access ctx directly without runtime freezing
    let ctx_module = Module::new();
    let heap = ctx_module.heap();
    let task_args = args_builder(heap);
    let task_info = TaskInfo {
        name: task.get_name(),
        group: task.get_group(),
    };
    let context = heap.alloc(TaskContext::new(task_args, config_value, task_info));
    ctx_module.set("__ctx__", context);

    let frozen_ctx_module = ctx_module
        .freeze()
        .map_err(|e| EvalError::UnknownError(anyhow!("{:?}", e)))?;
    // OwnedFrozenValue keeps the frozen heap alive for the duration of this function
    let frozen_context = frozen_ctx_module
        .get("__ctx__")
        .map_err(|e| EvalError::UnknownError(anyhow!("failed to get frozen context: {:?}", e)))?;

    // Create execution module for the evaluator
    let exec_module = Module::new();
    let mut eval = Evaluator::new(&exec_module);
    eval.extra = Some(&store);

    // Call frozen task implementation with frozen context
    let ret = eval.eval_function(task_impl.value(), &[frozen_context.value()], &[])?;

    Ok(ret.unpack_i32().map(|ex| ex as u8))
}

/// The core evaluator for .axl files.
#[derive(Debug)]
pub struct TaskEvaluator<'l, 'p> {
    loader: &'l AxlLoader<'p>,
}

impl<'l, 'p> TaskEvaluator<'l, 'p> {
    /// Creates a new TaskEvaluator with the given loader.
    pub fn new(loader: &'l AxlLoader<'p>) -> Self {
        Self { loader }
    }

    /// Evaluates the given .axl script path and immediately freezes the module.
    ///
    /// This is the preferred method following Buck2's pattern:
    /// - Modules are frozen immediately after evaluation
    /// - FrozenModule values can be safely stored and shared
    /// - Task execution uses temporary modules for the evaluation heap
    pub fn eval(&self, scope: ModuleScope, path: &Path) -> Result<FrozenModule, EvalError> {
        assert!(path.is_relative());
        let abs_path = join_confined(&scope.path, path)?;

        // push the current scope to stack
        self.loader.module_stack.borrow_mut().push(scope);

        let module = self.loader.eval_module(&abs_path)?;

        // pop the current scope off the stack
        let _scope = self
            .loader
            .module_stack
            .borrow_mut()
            .pop()
            .expect("just pushed a scope");

        // Freeze immediately
        module
            .freeze()
            .map_err(|e| EvalError::UnknownError(anyhow!(e)))
    }
}
