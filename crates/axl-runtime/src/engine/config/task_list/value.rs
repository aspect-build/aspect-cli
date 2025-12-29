use std::cell::RefCell;
use std::fmt;
use std::fmt::Debug;
use std::fmt::Display;

use allocative::Allocative;
use starlark::any::ProvidesStaticType;
use starlark::environment::Methods;
use starlark::environment::MethodsBuilder;
use starlark::environment::MethodsStatic;
use starlark::eval::Evaluator;
use starlark::starlark_module;
use starlark::typing::Ty;
use starlark::values::list::UnpackList;
use starlark::values::none::NoneType;
use starlark::values::starlark_value;
use starlark::values::type_repr::StarlarkTypeRepr;
use starlark::values::AllocValue;
use starlark::values::Heap;
use starlark::values::NoSerialize;
use starlark::values::StarlarkValue;
use starlark::values::Trace;
use starlark::values::Value;
use starlark::values::ValueLike;

use super::r#ref::TaskListMut;
use super::task_mut::TaskMut;

use crate::engine::store::AxlStore;

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
        #[starlark[require = pos]] task: Value<'v>,
        #[starlark[require = named]] name: String,
        #[starlark[require = named, default = UnpackList::default()]] group: UnpackList<String>,
        eval: &mut Evaluator<'v, '_, '_>,
    ) -> starlark::Result<NoneType> {
        let store = AxlStore::from_eval(eval)?;
        let mut this = TaskListMut::from_value(this)?;
        this.aref.content.push(eval.heap().alloc(TaskMut::new(
            eval.module(),
            store.script_path.to_string_lossy().to_string(),
            name,
            group.items,
            task,
        )));
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
#[repr(transparent)]
pub struct TaskList<'v> {
    pub(crate) content: Vec<Value<'v>>,
}

impl<'v> TaskList<'v> {
    pub fn new(content: Vec<Value<'v>>) -> Self {
        return TaskList { content };
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
