use crate::engine::task_context::TaskContext;

use super::task_arg::TaskArg;
use allocative::Allocative;
use derive_more::Display;
use starlark::collections::SmallMap;
use starlark::environment::GlobalsBuilder;
use starlark::eval::Evaluator;
use starlark::starlark_module;
use starlark::starlark_simple_value;
use starlark::typing::ParamIsRequired;
use starlark::typing::ParamSpec;
use starlark::values;
use starlark::values::list::UnpackList;
use starlark::values::none::NoneOr;
use starlark::values::none::NoneType;
use starlark::values::record::Record;
use starlark::values::starlark_value;
use starlark::values::typing::StarlarkCallableParamSpec;
use starlark::values::NoSerialize;
use starlark::values::ProvidesStaticType;
use starlark::values::StarlarkValue;
use starlark::values::Trace;
use starlark::values::Value;

pub const MAX_TASK_GROUPS: usize = 5;

pub trait TaskLike<'v>: 'v {
    fn args(&self) -> &SmallMap<String, TaskArg>;
    fn description(&self) -> &String;
    fn group(&self) -> &Vec<String>;
    fn name(&self) -> &String;
    fn binding(&self) -> values::Value<'v>;
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
#[display("<Task>")]
pub struct Task<'v> {
    r#impl: values::Value<'v>,
    binding: values::Value<'v>,
    #[allocative(skip)]
    pub(super) args: SmallMap<String, TaskArg>,
    pub(super) description: String,
    pub(super) group: Vec<String>,
    pub(super) name: String,
}

impl<'v> Task<'v> {
    pub fn implementation(&self) -> values::Value<'v> {
        self.r#impl
    }
    pub fn binding(&self) -> values::Value<'v> {
        self.binding
    }
    pub fn args(&self) -> &SmallMap<String, TaskArg> {
        &self.args
    }
    pub fn description(&self) -> &String {
        &self.description
    }
    pub fn group(&self) -> &Vec<String> {
        &self.group
    }
    pub fn name(&self) -> &String {
        &self.name
    }
}

impl<'v> TaskLike<'v> for Task<'v> {
    fn binding(&self) -> values::Value<'v> {
        self.binding
    }
    fn args(&self) -> &SmallMap<String, TaskArg> {
        &self.args
    }

    fn description(&self) -> &String {
        &self.description
    }
    fn group(&self) -> &Vec<String> {
        &self.group
    }
    fn name(&self) -> &String {
        &self.name
    }
}

#[starlark_value(type = "Task")]
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
        let binding = self.binding.freeze(freezer)?;
        Ok(FrozenTask {
            args: self.args,
            binding: binding,
            r#impl: frozen_impl,
            description: self.description,
            group: self.group,
            name: self.name,
        })
    }
}

#[derive(Debug, Display, Clone, ProvidesStaticType, Trace, NoSerialize, Allocative)]
#[display("<Task>")]
pub struct FrozenTask {
    r#impl: values::FrozenValue,
    binding: values::FrozenValue,
    #[allocative(skip)]
    pub(super) args: SmallMap<String, TaskArg>,
    pub(super) description: String,
    pub(super) group: Vec<String>,
    pub(super) name: String,
}

starlark_simple_value!(FrozenTask);

#[starlark_value(type = "Task")]
impl<'v> StarlarkValue<'v> for FrozenTask {
    type Canonical = Task<'v>;
}

impl FrozenTask {
    pub fn implementation(&self) -> values::FrozenValue {
        self.r#impl
    }
}

impl<'v> TaskLike<'v> for FrozenTask {
    fn binding(&self) -> values::Value<'v> {
        self.binding.to_value()
    }
    fn args(&self) -> &SmallMap<String, TaskArg> {
        &self.args
    }
    fn description(&self) -> &String {
        &self.description
    }
    fn group(&self) -> &Vec<String> {
        &self.group
    }
    fn name(&self) -> &String {
        &self.name
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
pub fn register_globals(globals: &mut GlobalsBuilder) {
    /// Task type representing a Task.
    ///
    /// ```python
    /// def _task_impl(ctx):
    ///     pass
    ///
    /// build = task(
    ///     name = "build",
    ///     group = [],
    ///     impl = _task_impl,
    ///     description = "build task",
    ///     task_args = {
    ///         "target": args.string(),
    ///     }
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
        #[starlark(require = named, default = UnpackList::default())] group: UnpackList<String>,
        #[starlark(require = named, default = NoneOr::None)] binding: NoneOr<values::Value<'v>>,
        #[starlark(require = named, default = String::new())] name: String,
        eval: &mut Evaluator<'v, '_, '_>,
    ) -> starlark::Result<Task<'v>> {
        if group.items.len() > MAX_TASK_GROUPS {
            return Err(anyhow::anyhow!(
                "task cannot have more than {} group levels",
                MAX_TASK_GROUPS
            )
            .into());
        }
        let binding = if let Some(binding) = binding.into_option() {
            eval.eval_function(binding, &[], &[])?
        } else {
            Value::new_none()
        };

        let mut args_ = SmallMap::new();
        for (arg, def) in args.entries {
            args_.insert(arg.to_owned(), def.clone());
        }
        Ok(Task {
            args: args_,
            binding,
            r#impl: implementation.0,
            description,
            group: group.items,
            name,
        })
    }
}
