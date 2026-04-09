use crate::engine::task_context::TaskContext;
use crate::engine::types::r#trait::{FrozenTraitType, TraitType, extract_trait_type_id};

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
use starlark::values::NoSerialize;
use starlark::values::ProvidesStaticType;
use starlark::values::StarlarkValue;
use starlark::values::Trace;
use starlark::values::Value;
use starlark::values::ValueLike;
use starlark::values::list::UnpackList;
use starlark::values::none::NoneType;
use starlark::values::starlark_value;
use starlark::values::typing::StarlarkCallableParamSpec;

pub const MAX_TASK_GROUPS: usize = 5;

pub trait TaskLike<'v>: 'v {
    fn args(&self) -> &SmallMap<String, TaskArg>;
    fn description(&self) -> &String;
    fn group(&self) -> &Vec<String>;
    fn name(&self) -> &String;
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
    #[allocative(skip)]
    pub(super) args: SmallMap<String, TaskArg>,
    pub(super) description: String,
    pub(super) group: Vec<String>,
    pub(super) name: String,
    pub(super) traits: Vec<values::Value<'v>>,
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
    pub fn group(&self) -> &Vec<String> {
        &self.group
    }
    pub fn name(&self) -> &String {
        &self.name
    }
    pub fn traits(&self) -> &[values::Value<'v>] {
        &self.traits
    }
}

impl<'v> TaskLike<'v> for Task<'v> {
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
    fn alloc_value(self, heap: values::Heap<'v>) -> Value<'v> {
        heap.alloc_complex(self)
    }
}

impl<'v> values::Freeze for Task<'v> {
    type Frozen = FrozenTask;
    fn freeze(self, freezer: &values::Freezer) -> values::FreezeResult<Self::Frozen> {
        let frozen_impl = self.r#impl.freeze(freezer)?;
        let frozen_traits: Result<Vec<_>, _> =
            self.traits.iter().map(|f| f.freeze(freezer)).collect();
        Ok(FrozenTask {
            args: self.args,
            r#impl: frozen_impl,
            description: self.description,
            group: self.group,
            name: self.name,
            traits: frozen_traits?,
        })
    }
}

#[derive(Debug, Display, Clone, ProvidesStaticType, Trace, NoSerialize, Allocative)]
#[display("<Task>")]
pub struct FrozenTask {
    r#impl: values::FrozenValue,
    #[allocative(skip)]
    pub(super) args: SmallMap<String, TaskArg>,
    pub(super) description: String,
    pub(super) group: Vec<String>,
    pub(super) name: String,
    pub(super) traits: Vec<values::FrozenValue>,
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
    pub fn traits(&self) -> &[values::FrozenValue] {
        &self.traits
    }
    /// Get trait type IDs this task opts into.
    pub fn trait_type_ids(&self) -> Vec<u64> {
        self.traits
            .iter()
            .filter_map(|f| extract_trait_type_id(f.to_value()))
            .collect()
    }
}

impl<'v> TaskLike<'v> for FrozenTask {
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
    ///     },
    ///     traits = [BazelTrait]  # Optional list of trait types
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
        #[starlark(require = named, default = String::new())] name: String,
        #[starlark(require = named, default = UnpackList::default())] traits: UnpackList<Value<'v>>,
    ) -> anyhow::Result<Task<'v>> {
        if group.items.len() > MAX_TASK_GROUPS {
            return Err(anyhow::anyhow!(
                "task cannot have more than {} group levels",
                MAX_TASK_GROUPS
            )
            .into());
        }
        let mut args_ = SmallMap::new();
        for (arg, def) in args.entries {
            args_.insert(arg.to_owned(), def.clone());
        }

        // Validate each element is a TraitType or FrozenTraitType
        let all_traits = traits.items;
        for t in &all_traits {
            if t.downcast_ref::<TraitType>().is_none()
                && t.downcast_ref::<FrozenTraitType>().is_none()
            {
                return Err(anyhow::anyhow!(
                    "traits list must contain trait types, got '{}'",
                    t.get_type()
                )
                .into());
            }
        }

        Ok(Task {
            args: args_,
            r#impl: implementation.0,
            description,
            group: group.items,
            name,
            traits: all_traits,
        })
    }
}
