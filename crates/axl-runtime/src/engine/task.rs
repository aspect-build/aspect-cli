use crate::engine::task_context::TaskContext;

use super::task_arg::TaskArg;
use allocative::Allocative;
use derive_more::Display;
use starlark::collections::SmallMap;
use starlark::environment::GlobalsBuilder;
use starlark::starlark_module;
use starlark::starlark_simple_value;
use starlark::typing::ParamIsRequired;
use starlark::typing::ParamSpec;
use starlark::values;
use starlark::values::list::UnpackList;
use starlark::values::none::NoneType;
use starlark::values::starlark_value;
use starlark::values::typing::StarlarkCallableParamSpec;
use starlark::values::NoSerialize;
use starlark::values::ProvidesStaticType;
use starlark::values::StarlarkValue;
use starlark::values::Trace;
use starlark::values::Value;

pub trait TaskLike<'v>: 'v {
    fn args(&self) -> &SmallMap<String, TaskArg>;
    fn description(&self) -> &String;
    fn groups(&self) -> &Vec<String>;
}

pub trait AsTaskLike<'v>: TaskLike<'v> {
    fn as_task(&self) -> &dyn TaskLike<'v>;
}

impl<'v, T> AsTaskLike<'v> for T
where
    T: TaskLike<'v>,
{
    fn as_task(&self) -> &dyn TaskLike<'v> {
        self
    }
}

#[derive(Debug, Clone, ProvidesStaticType, Display, Trace, NoSerialize, Allocative)]
#[display("<task>")]
pub struct Task<'v> {
    r#impl: values::Value<'v>,
    #[allocative(skip)]
    args: SmallMap<String, TaskArg>,
    description: String,
    groups: Vec<String>,
}

impl<'v> Task<'v> {
    pub fn implementation(&self) -> values::Value<'v> {
        self.r#impl
    }
    pub fn args(&self) -> &SmallMap<String, TaskArg> {
        &self.args
    }
    pub fn description(&self) -> &String {
        &self.description
    }
    pub fn groups(&self) -> &Vec<String> {
        &self.groups
    }
}

impl<'v> TaskLike<'v> for Task<'v> {
    fn args(&self) -> &SmallMap<String, TaskArg> {
        &self.args
    }
    fn description(&self) -> &String {
        &self.description
    }
    fn groups(&self) -> &Vec<String> {
        &self.groups
    }
}

#[starlark_value(type = "task")]
impl<'v> StarlarkValue<'v> for Task<'v> {}

impl<'v> values::AllocValue<'v> for Task<'v> {
    fn alloc_value(self, heap: &'v values::Heap) -> Value<'v> {
        heap.alloc_complex(self)
    }
}

impl<'v> values::Freeze for Task<'v> {
    type Frozen = FrozenTask;
    fn freeze(self, freezer: &values::Freezer) -> values::FreezeResult<Self::Frozen> {
        let frozen_impl = self.r#impl.freeze(freezer)?;
        Ok(FrozenTask {
            args: self.args,
            r#impl: frozen_impl,
            description: self.description,
            groups: self.groups,
        })
    }
}

#[derive(Debug, Display, ProvidesStaticType, NoSerialize, Allocative)]
#[display("<task>")]
pub struct FrozenTask {
    r#impl: values::FrozenValue,
    #[allocative(skip)]
    args: SmallMap<String, TaskArg>,
    description: String,
    groups: Vec<String>,
}

starlark_simple_value!(FrozenTask);

#[starlark_value(type = "task")]
impl<'v> StarlarkValue<'v> for FrozenTask {
    type Canonical = Task<'v>;
}

impl FrozenTask {
    pub fn implementation(&self) -> values::FrozenValue {
        self.r#impl
    }
}

impl<'v> TaskLike<'v> for FrozenTask {
    fn args(&self) -> &SmallMap<String, TaskArg> {
        &self.args
    }
    fn description(&self) -> &String {
        &self.description
    }
    fn groups(&self) -> &Vec<String> {
        &self.groups
    }
}

struct TaskImpl;

impl StarlarkCallableParamSpec for TaskImpl {
    fn params() -> ParamSpec {
        ParamSpec::new_parts(
            [(ParamIsRequired::Yes, TaskContext::get_type_starlark_repr())],
            [],
            None,
            [],
            None,
        )
        .unwrap()
    }
}

#[starlark_module]
pub fn register_toplevels(_: &mut GlobalsBuilder) {
    /// Task type representing a Task.
    ///
    /// ```python
    /// def _task_impl(ctx):
    ///     pass
    ///
    /// build = task(
    ///     impl = _task_impl,
    ///     task_args = {
    ///         "target": args.string(),
    ///     }
    ///     groups = [],
    /// )
    /// ```
    fn task<'v>(
        #[starlark(require = named)] implementation: values::typing::StarlarkCallable<
            'v,
            TaskImpl,
            NoneType,
        >,
        #[starlark(require = named)] args: values::dict::UnpackDictEntries<&'v str, TaskArg>,
        #[starlark(require = named, default = String::new())] description: String,
        #[starlark(require = named, default = UnpackList::default())] groups: UnpackList<String>,
    ) -> starlark::Result<Task<'v>> {
        let mut args_ = SmallMap::new();
        for (arg, def) in args.entries {
            args_.insert(arg.to_owned(), def.clone());
        }
        Ok(Task {
            args: args_,
            r#impl: implementation.0,
            description,
            groups: groups.items,
        })
    }
}
