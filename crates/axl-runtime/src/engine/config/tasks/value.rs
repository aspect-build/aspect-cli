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
use starlark::values::Freeze;
use starlark::values::FreezeError;
use starlark::values::Freezer;
use starlark::values::FrozenValue;
use starlark::values::Heap;
use starlark::values::NoSerialize;
use starlark::values::StarlarkValue;
use starlark::values::Trace;
use starlark::values::Tracer;
use starlark::values::Value;
use starlark::values::ValueLike;
use starlark::values::none::NoneType;
use starlark::values::starlark_value;
use starlark::values::type_repr::StarlarkTypeRepr;

use super::configured_task::ConfiguredTask;
use super::r#ref::TaskListMut;

use crate::engine::config::trait_map::TraitMap;
use crate::engine::store::AxlStore;
use crate::engine::task::{AsTaskLike, FrozenTask, Task, TaskLike};
use crate::engine::types::r#trait::extract_trait_type_id;

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
    ) -> anyhow::Result<NoneType> {
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
        let (_frozen_module, task_def) = Module::with_temp_heap(|temp_module| {
            let short_task: Value = unsafe { std::mem::transmute(task) };
            temp_module.set(&symbol, short_task);
            let frozen = temp_module
                .freeze()
                .map_err(|e| anyhow::anyhow!("failed to freeze task: {:?}", e))?;
            let task_def = frozen
                .get(&symbol)
                .map_err(|e| anyhow::anyhow!("failed to get frozen task: {:?}", e))?;
            Ok::<_, anyhow::Error>((frozen, task_def))
        })?;

        // Get trait type IDs from the frozen task
        let frozen_task = task_def
            .value()
            .downcast_ref::<FrozenTask>()
            .ok_or_else(|| anyhow::anyhow!("expected FrozenTask after freeze"))?;
        let trait_type_ids = frozen_task.trait_type_ids();

        // Auto-register any new trait types into the TraitMap
        if let Some(fmap_value) = this.aref.trait_map {
            if let Some(fmap) = fmap_value.downcast_ref::<TraitMap>() {
                for trait_fv in frozen_task.traits() {
                    let trait_value = trait_fv.to_value();
                    if let Some(id) = extract_trait_type_id(trait_value) {
                        if !fmap.contains(id) {
                            // Auto-construct default instance by calling the trait type with no args
                            let instance =
                                eval.eval_function(trait_value, &[], &[]).map_err(|e| {
                                    anyhow::anyhow!(
                                        "Failed to construct default trait instance for {}: {:?}",
                                        trait_value,
                                        e
                                    )
                                })?;
                            fmap.insert(id, trait_value, instance);
                        }
                    }
                }
            }
        }

        // Create ConfiguredTask with trait type IDs
        let task_mut = ConfiguredTask::new_with_traits(
            task_def,
            name,
            task_like.group().to_vec(),
            trait_type_ids,
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

    fn iterate_collect(&self, heap: Heap<'v>) -> starlark::Result<Vec<Value<'v>>> {
        self.0
            .iterate_collect(heap)
            .map_err(starlark::Error::new_other)
    }
}

pub(crate) type MutableTaskList<'v> = TaskListGen<RefCell<TaskList<'v>>>;

/// Unfrozen TaskList
#[derive(Clone, Trace, Debug, ProvidesStaticType, Allocative)]
pub struct TaskList<'v> {
    pub(crate) content: Vec<Value<'v>>,
    /// Optional reference to the TraitMap for auto-registering traits
    /// when tasks are added dynamically via ctx.tasks.add().
    #[allocative(skip)]
    pub(crate) trait_map: Option<Value<'v>>,
}

impl<'v> TaskList<'v> {
    pub fn new(content: Vec<Value<'v>>) -> Self {
        TaskList {
            content,
            trait_map: None,
        }
    }

    pub fn new_with_trait_map(content: Vec<Value<'v>>, trait_map: Value<'v>) -> Self {
        TaskList {
            content,
            trait_map: Some(trait_map),
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
    fn alloc_value(self, heap: Heap<'v>) -> Value<'v> {
        heap.alloc_complex(TaskListGen(RefCell::new(self)))
    }
}

/// Frozen task list data — holds frozen values after module freeze.
#[derive(Debug, ProvidesStaticType, Allocative)]
pub(crate) struct FrozenTaskListData {
    content: Vec<FrozenValue>,
    trait_map: Option<FrozenValue>,
}

impl fmt::Display for FrozenTaskListData {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "tasks")
    }
}

unsafe impl<'v> Trace<'v> for FrozenTaskListData {
    fn trace(&mut self, _tracer: &Tracer<'v>) {}
}

impl<'v> Freeze for TaskListGen<RefCell<TaskList<'v>>> {
    type Frozen = TaskListGen<FrozenTaskListData>;

    fn freeze(self, freezer: &Freezer) -> Result<Self::Frozen, FreezeError> {
        let inner = self.0.into_inner();
        let content = inner
            .content
            .into_iter()
            .map(|v| v.freeze(freezer))
            .collect::<Result<Vec<FrozenValue>, FreezeError>>()?;
        let trait_map = inner.trait_map.map(|v| v.freeze(freezer)).transpose()?;
        Ok(TaskListGen(FrozenTaskListData { content, trait_map }))
    }
}

trait TaskListLike<'v>: Debug + Allocative {
    fn iterate_collect(&self, _heap: Heap<'v>) -> anyhow::Result<Vec<Value<'v>>>;
}

impl<'v> TaskListLike<'v> for RefCell<TaskList<'v>> {
    fn iterate_collect(&self, _heap: Heap<'v>) -> anyhow::Result<Vec<Value<'v>>> {
        Ok(self
            .borrow()
            .content
            .iter()
            .map(|f| f.to_value())
            .collect::<Vec<Value<'v>>>())
    }
}

impl<'v> TaskListLike<'v> for FrozenTaskListData {
    fn iterate_collect(&self, _heap: Heap<'v>) -> anyhow::Result<Vec<Value<'v>>> {
        Ok(self.content.iter().map(|f| f.to_value()).collect())
    }
}
