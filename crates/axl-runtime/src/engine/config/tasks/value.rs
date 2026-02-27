use std::cell::RefCell;
use std::fmt;
use std::fmt::Debug;
use std::fmt::Display;
use std::path::PathBuf;

use allocative::Allocative;
use starlark::any::ProvidesStaticType;
use starlark::environment::Methods;
use starlark::environment::MethodsBuilder;
use starlark::environment::MethodsStatic;
use starlark::environment::Module;
use starlark::eval::Evaluator;
use starlark::starlark_module;
use starlark::typing::Ty;
use starlark::values::AllocValue;
use starlark::values::Heap;
use starlark::values::NoSerialize;
use starlark::values::StarlarkValue;
use starlark::values::Trace;
use starlark::values::Value;
use starlark::values::ValueLike;
use starlark::values::none::NoneType;
use starlark::values::starlark_value;
use starlark::values::type_repr::StarlarkTypeRepr;

use super::configured_task::ConfiguredTask;
use super::r#ref::TaskListMut;

use crate::engine::config::fragment_map::FragmentMap;
use crate::engine::store::AxlStore;
use crate::engine::task::{AsTaskLike, FrozenTask, Task, TaskLike};
use crate::engine::types::fragment::extract_fragment_type_id;

#[derive(Clone, Default, Trace, Debug, ProvidesStaticType, NoSerialize, Allocative)]
pub(crate) struct TaskListGen<T>(pub(crate) T);

impl<'v, T: TaskListLike<'v>> Display for TaskListGen<T> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "tasks")
    }
}

#[starlark_module]
pub(crate) fn task_list_methods(registry: &mut MethodsBuilder) {
    fn add<'v>(
        #[allow(unused)] this: Value<'v>,
        #[starlark(require = pos)] task: Value<'v>,
        eval: &mut Evaluator<'v, '_, '_>,
    ) -> starlark::Result<NoneType> {
        let store = AxlStore::from_eval(eval)?;
        let mut this = TaskListMut::from_value(this)?;
        let symbol = format!("__added_task_{}", this.aref.content.len());

        // Get task metadata
        let task_like: &dyn TaskLike = if let Some(t) = task.downcast_ref::<Task>() {
            t.as_task()
        } else if let Some(t) = task.downcast_ref::<FrozenTask>() {
            t.as_task()
        } else {
            return Err(anyhow::anyhow!(
                "expected value of type 'Task', got '{}'",
                task.get_type()
            )
            .into());
        };

        let name = task_like.name().to_owned();
        if name.is_empty() {
            return Err(anyhow::anyhow!("Task name required").into());
        }

        // Freeze the task value to create OwnedFrozenValue
        let temp_module = Module::new();
        let short_task: Value = unsafe { std::mem::transmute(task) };
        temp_module.set(&symbol, short_task);
        let frozen = temp_module
            .freeze()
            .map_err(|e| anyhow::anyhow!("failed to freeze task: {:?}", e))?;
        let task_def = frozen
            .get(&symbol)
            .map_err(|e| anyhow::anyhow!("failed to get frozen task: {:?}", e))?;

        // Get fragment type IDs from the frozen task
        let frozen_task = task_def
            .value()
            .downcast_ref::<FrozenTask>()
            .ok_or_else(|| anyhow::anyhow!("expected FrozenTask after freeze"))?;
        let fragment_type_ids = frozen_task.fragment_type_ids();

        // Auto-register any new fragment types into the FragmentMap
        if let Some(fmap_value) = this.aref.fragment_map {
            if let Some(fmap) = fmap_value.downcast_ref::<FragmentMap>() {
                for frag_fv in frozen_task.fragments() {
                    let frag_value = frag_fv.to_value();
                    if let Some(id) = extract_fragment_type_id(frag_value) {
                        if !fmap.contains(id) {
                            // Auto-construct default instance by calling the fragment type with no args
                            let instance = eval.eval_function(frag_value, &[], &[]).map_err(|e| {
                                anyhow::anyhow!(
                                    "Failed to construct default fragment instance for {}: {:?}",
                                    frag_value,
                                    e
                                )
                            })?;
                            fmap.insert(id, frag_value, instance);
                        }
                    }
                }
            }
        }

        // Create ConfiguredTask with fragment type IDs
        let task_mut = ConfiguredTask::new_with_fragments(
            task_def,
            name,
            task_like.group().to_vec(),
            fragment_type_ids,
            symbol,
            PathBuf::from(store.script_path.to_string_lossy().to_string()),
        );

        this.aref.content.push(eval.heap().alloc(task_mut));
        Ok(NoneType)
    }
}

#[starlark_value(type = "tasks")]
impl<'v, T: TaskListLike<'v> + 'v> StarlarkValue<'v> for TaskListGen<T>
where
    Self: ProvidesStaticType<'v>,
{
    type Canonical = Self;

    fn get_methods() -> Option<&'static Methods> {
        static RES: MethodsStatic = MethodsStatic::new();
        RES.methods(task_list_methods)
    }

    fn iterate_collect(&self, heap: &'v Heap) -> starlark::Result<Vec<Value<'v>>> {
        self.0.iterate_collect(heap)
    }
}

pub(crate) type MutableTaskList<'v> = TaskListGen<RefCell<TaskList<'v>>>;

/// Unfrozen TaskList
#[derive(Clone, Trace, Debug, ProvidesStaticType, Allocative)]
pub struct TaskList<'v> {
    pub(crate) content: Vec<Value<'v>>,
    /// Optional reference to the FragmentMap for auto-registering fragments
    /// when tasks are added dynamically via ctx.tasks.add().
    #[allocative(skip)]
    pub(crate) fragment_map: Option<Value<'v>>,
}

impl<'v> TaskList<'v> {
    pub fn new(content: Vec<Value<'v>>) -> Self {
        TaskList {
            content,
            fragment_map: None,
        }
    }

    pub fn new_with_fragment_map(content: Vec<Value<'v>>, fragment_map: Value<'v>) -> Self {
        TaskList {
            content,
            fragment_map: Some(fragment_map),
        }
    }
}

impl<'v> Display for TaskList<'v> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "tasks")
    }
}

impl<'v> StarlarkTypeRepr for TaskList<'v> {
    type Canonical = Self;
    fn starlark_type_repr() -> Ty {
        Ty::any()
    }
}

impl<'v> AllocValue<'v> for TaskList<'v> {
    fn alloc_value(self, heap: &'v Heap) -> Value<'v> {
        heap.alloc_complex_no_freeze(TaskListGen(RefCell::new(self)))
    }
}

trait TaskListLike<'v>: Debug + Allocative {
    fn iterate_collect(&self, _heap: &'v Heap) -> starlark::Result<Vec<Value<'v>>>;
}

impl<'v> TaskListLike<'v> for RefCell<TaskList<'v>> {
    fn iterate_collect(&self, _heap: &'v Heap) -> starlark::Result<Vec<Value<'v>>> {
        Ok(self
            .borrow()
            .content
            .iter()
            .map(|f| f.to_value())
            .collect::<Vec<Value<'v>>>())
    }
}
