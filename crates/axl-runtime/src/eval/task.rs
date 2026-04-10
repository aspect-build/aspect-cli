use anyhow::anyhow;
use starlark::environment::FrozenModule;
use starlark::environment::Module;
use starlark::eval::Evaluator;
use starlark::values::FrozenValue;
use starlark::values::Heap;
use starlark::values::OwnedFrozenValue;
use starlark::values::Value;
use starlark::values::ValueLike;
use starlark::values::list::AllocList;
use std::path::Path;
use uuid::Uuid;

use crate::engine::bazel::Bazel;
use crate::engine::config::ConfiguredTask;
use crate::engine::config::trait_map::TraitMap;
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

/// Build a task-scoped TraitMap on the given heap, containing only the
/// traits the task opts into.
fn build_task_trait_map<'v>(
    trait_data: &[(u64, Value<'static>, Value<'static>)],
    task_trait_ids: &[u64],
    heap: Heap<'v>,
) -> Value<'v> {
    let map = TraitMap::new();
    for (id, type_val, instance_val) in trait_data {
        if task_trait_ids.contains(id) {
            // SAFETY: trait_data values live on a leaked heap that outlives this call
            let tv: Value<'v> = unsafe { std::mem::transmute(*type_val) };
            let iv: Value<'v> = unsafe { std::mem::transmute(*instance_val) };
            map.insert(*id, tv, iv);
        }
    }
    heap.alloc(map)
}

/// Executes a task with pre-built TaskArgs.
///
/// The TaskContext is allocated on the execution heap (unfrozen), so mutable
/// Starlark values like `ctx.bazel.startup_flags` can be appended to during execution.
pub fn execute_task_with_args(
    task: &ConfiguredTask,
    store: AxlStore,
    trait_data: &[(u64, Value<'static>, Value<'static>)],
    task_key: String,
    task_key_is_generated: bool,
    task_id: Option<String>,
    args_builder: impl FnOnce(Heap) -> TaskArgs,
) -> Result<Option<u8>, EvalError> {
    // Get the task implementation function
    let task_impl = task
        .implementation()
        .ok_or_else(|| EvalError::UnknownError(anyhow!("task has no implementation")))?;

    // Extract FrozenValue (Copy, 'static-compatible) for use inside with_temp_heap closure.
    // The OwnedFrozenValue keeps the heap alive for the duration of this function.
    let task_impl_fv: FrozenValue = unsafe { task_impl.unchecked_frozen_value() };

    let task_id = task_id.unwrap_or_else(|| Uuid::new_v4().to_string());

    // TODO: move all of the following output to AXL task lifecycle hooks once
    // ctx.task.lifecycle_hooks (pre_task / post_task) is implemented.
    // At that point, features can register hooks to:
    //   - print a Buildkite section header ("--- :aspect: Running <name> (<key>)")
    //     by checking ctx.std.env CI host (e.g. environment.ci.host == "buildkite")
    //   - print the invocation line ("aspect: <name> task invocation (key: ...) (id: ...)")
    //   - print the --task-key tip on CI when the key was auto-generated
    //     (ctx.task.key and a flag indicating whether it was user-supplied would
    //     need to be exposed, or the tip logic moved entirely into AXL)
    // For now these are printed unconditionally from Rust before task execution.

    // Single execution heap — allocate TaskContext directly (no pre-freeze needed).
    Module::with_temp_heap(|exec_module| {
        let heap = exec_module.heap();
        let task_args = args_builder(heap);
        let task_info = TaskInfo {
            name: task.get_name(),
            group: task.get_group(),
            task_key: task_key.clone(),
            task_id: task_id.clone(),
        };

        // Build a task-scoped trait map
        let trait_map = build_task_trait_map(trait_data, &task.trait_type_ids, heap);

        let startup_flags = heap.alloc(AllocList([] as [String; 0]));
        let bazel = heap.alloc(Bazel { startup_flags });
        let context = heap.alloc(TaskContext::new(task_args, trait_map, task_info, bazel));

        let mut eval = Evaluator::new(&exec_module);
        eval.extra = Some(&store);

        let ret = eval.eval_function(task_impl_fv.to_value(), &[context], &[])?;

        Ok(ret.unpack_i32().map(|ex| ex as u8))
    })
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

        let frozen = self.loader.eval_module(&abs_path)?;

        // pop the current scope off the stack
        let _scope = self
            .loader
            .module_stack
            .borrow_mut()
            .pop()
            .expect("just pushed a scope");

        // Cache the frozen module so that subsequent load() calls for the same
        // path (e.g., from config files) return this module instead of
        // re-evaluating and creating new type instances with different IDs.
        self.loader.cache_module(abs_path, frozen.clone());

        Ok(frozen)
    }
}
