use std::cell::RefCell;
use std::fmt;
use std::path::PathBuf;

use allocative::Allocative;
use starlark::any::ProvidesStaticType;
use starlark::environment::Methods;
use starlark::environment::MethodsBuilder;
use starlark::environment::MethodsStatic;
use starlark::eval::Evaluator;
use starlark::starlark_module;
use starlark::starlark_simple_value;
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

use super::configured_task::ConfiguredTask;
use crate::engine::store::AxlStore;
use crate::engine::task::{FrozenTask, TaskLike};

/// Live (mutable) task list, exposed to Starlark as `ctx.tasks`.
#[derive(Debug, ProvidesStaticType, NoSerialize, Allocative)]
pub struct TaskList<'v> {
    #[allocative(skip)]
    pub(crate) content: RefCell<Vec<Value<'v>>>,
}

impl<'v> fmt::Display for TaskList<'v> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "tasks")
    }
}

unsafe impl<'v> Trace<'v> for TaskList<'v> {
    fn trace(&mut self, tracer: &Tracer<'v>) {
        for v in self.content.get_mut().iter_mut() {
            v.trace(tracer);
        }
    }
}

impl<'v> AllocValue<'v> for TaskList<'v> {
    fn alloc_value(self, heap: Heap<'v>) -> Value<'v> {
        heap.alloc_complex(self)
    }
}

impl<'v> Freeze for TaskList<'v> {
    type Frozen = FrozenTaskList;

    fn freeze(self, freezer: &Freezer) -> Result<FrozenTaskList, FreezeError> {
        let content = self
            .content
            .into_inner()
            .into_iter()
            .map(|v| v.freeze(freezer))
            .collect::<Result<Vec<FrozenValue>, FreezeError>>()?;
        Ok(FrozenTaskList { content })
    }
}

impl<'v> TaskList<'v> {
    pub fn new(content: Vec<Value<'v>>) -> Self {
        TaskList {
            content: RefCell::new(content),
        }
    }

    /// Borrow the task values for iteration.
    pub fn values(&self) -> Vec<Value<'v>> {
        self.content.borrow().clone()
    }
}

#[starlark_value(type = "tasks")]
impl<'v> StarlarkValue<'v> for TaskList<'v> {
    fn get_methods() -> Option<&'static Methods> {
        static RES: MethodsStatic = MethodsStatic::new();
        RES.methods(task_list_methods)
    }

    fn iterate_collect(&self, _heap: Heap<'v>) -> starlark::Result<Vec<Value<'v>>> {
        Ok(self.content.borrow().clone())
    }
}

#[starlark_module]
fn task_list_methods(registry: &mut MethodsBuilder) {
    /// Add a task to the task list.
    ///
    /// The task value must be a `FrozenTask` (i.e. defined at module level, not inside
    /// a function). Since config files are frozen before their `config()` function is
    /// called, module-level task definitions are always frozen by the time `add` runs.
    fn add<'v>(
        this: Value<'v>,
        #[starlark(require = pos)] task: Value<'v>,
        eval: &mut Evaluator<'v, '_, '_>,
    ) -> anyhow::Result<NoneType> {
        let store = AxlStore::from_eval(eval)?;

        let frozen_task = task.downcast_ref::<FrozenTask>().ok_or_else(|| {
            anyhow::anyhow!(
                "ctx.tasks.add: expected a task() value, got '{}'",
                task.get_type()
            )
        })?;

        let name = frozen_task.name().to_owned();
        if name.is_empty() {
            return Err(anyhow::anyhow!("ctx.tasks.add: task must have a non-empty name").into());
        }

        let configured = ConfiguredTask::new_with_traits(
            task.unpack_frozen().expect("FrozenTask is always frozen"),
            name,
            frozen_task.group().to_vec(),
            frozen_task.trait_type_ids(),
            frozen_task.name().to_owned(),
            PathBuf::from(store.script_path.to_string_lossy().as_ref()),
        );

        this.downcast_ref::<TaskList>()
            .ok_or_else(|| anyhow::anyhow!("ctx.tasks is not a task list"))?
            .content
            .try_borrow_mut()
            .map_err(|_| anyhow::anyhow!("cannot mutate ctx.tasks while iterating over it"))?
            .push(eval.heap().alloc(configured));

        Ok(NoneType)
    }
}

/// Frozen task list — read-only after freeze.
#[derive(Debug, ProvidesStaticType, NoSerialize, Allocative)]
pub struct FrozenTaskList {
    content: Vec<FrozenValue>,
}

impl fmt::Display for FrozenTaskList {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "tasks")
    }
}

unsafe impl<'v> Trace<'v> for FrozenTaskList {
    fn trace(&mut self, _tracer: &Tracer<'v>) {}
}

starlark_simple_value!(FrozenTaskList);

#[starlark_value(type = "tasks")]
impl<'v> StarlarkValue<'v> for FrozenTaskList {
    type Canonical = TaskList<'v>;

    fn iterate_collect(&self, _heap: Heap<'v>) -> starlark::Result<Vec<Value<'v>>> {
        Ok(self.content.iter().map(|v| v.to_value()).collect())
    }
}
