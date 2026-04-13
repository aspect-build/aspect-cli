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

use super::configured_task::{ConfiguredTask, FrozenConfiguredTask};
use crate::engine::store::AxlStore;
use crate::engine::task::{FrozenTask, TaskLike};

/// Compute the canonical path key for a task.
///
/// - group=`[]`, name=`"bar"` → `"bar"`
/// - group=`["foo"]`, name=`"bar"` → `"foo/bar"`
/// - group=`["foo","fum"]`, name=`"bar"` → `"foo/fum/bar"`
pub fn task_key(group: &[String], name: &str) -> String {
    if group.is_empty() {
        name.to_owned()
    } else {
        format!("{}/{}", group.join("/"), name)
    }
}

/// Live (mutable) task map, exposed to Starlark as `ctx.tasks` in `config.axl`.
///
/// Supports both iteration (`for t in ctx.tasks`) and map-style key access
/// (`ctx.tasks["group/name"]`). The key is the task's canonical path:
/// a root-level task named `"foo"` is at `"foo"`; a task named `"bar"` in
/// group `["grp", "sub"]` is at `"grp/sub/bar"`. See `task_key()`.
///
/// Use `ctx.tasks.add(my_task)` to register a new task from `config.axl`.
/// Use `ctx.tasks["key"].args.x = val` to override a task arg.
#[derive(Debug, ProvidesStaticType, NoSerialize, Allocative)]
pub struct TaskMap<'v> {
    #[allocative(skip)]
    pub(crate) content: RefCell<Vec<Value<'v>>>,
}

impl<'v> fmt::Display for TaskMap<'v> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "tasks")
    }
}

unsafe impl<'v> Trace<'v> for TaskMap<'v> {
    fn trace(&mut self, tracer: &Tracer<'v>) {
        for v in self.content.get_mut().iter_mut() {
            v.trace(tracer);
        }
    }
}

impl<'v> AllocValue<'v> for TaskMap<'v> {
    fn alloc_value(self, heap: Heap<'v>) -> Value<'v> {
        heap.alloc_complex(self)
    }
}

impl<'v> Freeze for TaskMap<'v> {
    type Frozen = FrozenTaskMap;

    fn freeze(self, freezer: &Freezer) -> Result<FrozenTaskMap, FreezeError> {
        let content = self
            .content
            .into_inner()
            .into_iter()
            .map(|v| v.freeze(freezer))
            .collect::<Result<Vec<FrozenValue>, FreezeError>>()?;
        Ok(FrozenTaskMap { content })
    }
}

impl<'v> TaskMap<'v> {
    pub fn new(content: Vec<Value<'v>>) -> Self {
        TaskMap {
            content: RefCell::new(content),
        }
    }

    /// Borrow the task values for iteration.
    pub fn values(&self) -> Vec<Value<'v>> {
        self.content.borrow().clone()
    }
}

#[starlark_value(type = "tasks")]
impl<'v> StarlarkValue<'v> for TaskMap<'v> {
    fn get_methods() -> Option<&'static Methods> {
        static RES: MethodsStatic = MethodsStatic::new();
        RES.methods(task_map_methods)
    }

    fn iterate_collect(&self, _heap: Heap<'v>) -> starlark::Result<Vec<Value<'v>>> {
        Ok(self.content.borrow().clone())
    }

    fn at(&self, index: Value<'v>, _heap: Heap<'v>) -> starlark::Result<Value<'v>> {
        let key = index.unpack_str().ok_or_else(|| {
            starlark::Error::new_other(anyhow::anyhow!(
                "ctx.tasks key must be a string, got {}",
                index.get_type()
            ))
        })?;
        for val in self.content.borrow().iter() {
            if let Some(ct) = val.downcast_ref::<ConfiguredTask>() {
                if task_key(&ct.get_group(), &ct.get_name()) == key {
                    return Ok(*val);
                }
            }
        }
        Err(starlark::Error::new_other(anyhow::anyhow!(
            "no task found at path {:?}",
            key
        )))
    }
}

#[starlark_module]
fn task_map_methods(registry: &mut MethodsBuilder) {
    /// Add a task to the task map.
    ///
    /// The task value must be a `FrozenTask` (i.e. defined at module level, not inside a
    /// function). Since config files are frozen before their `config()` function is called,
    /// module-level task definitions are always frozen by the time `add` runs.
    ///
    /// After adding, the task is accessible via iteration and by key:
    /// `ctx.tasks["group/name"].args.x = val`.
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

        let key = task_key(&frozen_task.group(), &name);
        {
            let content = this
                .downcast_ref::<TaskMap>()
                .ok_or_else(|| anyhow::anyhow!("ctx.tasks is not a task map"))?
                .content
                .try_borrow()
                .map_err(|_| anyhow::anyhow!("cannot read ctx.tasks while mutating it"))?;
            if content.iter().any(|v| {
                v.downcast_ref::<ConfiguredTask>()
                    .map(|ct| task_key(&ct.get_group(), &ct.get_name()) == key)
                    .unwrap_or(false)
            }) {
                return Err(anyhow::anyhow!("ctx.tasks.add: task {:?} already exists", key).into());
            }
        }

        // `symbol` and `name` start as the same string; name is mutable (config.axl can
        // override it) while symbol stays fixed as the Starlark variable name.
        let symbol = name.clone();
        let configured = ConfiguredTask::new_with_traits(
            task.unpack_frozen().expect("FrozenTask is always frozen"),
            name,
            frozen_task.group().to_vec(),
            frozen_task.trait_type_ids(),
            symbol,
            PathBuf::from(store.script_path.to_string_lossy().as_ref()),
        );

        this.downcast_ref::<TaskMap>()
            .ok_or_else(|| anyhow::anyhow!("ctx.tasks is not a task map"))?
            .content
            .try_borrow_mut()
            .map_err(|_| anyhow::anyhow!("cannot mutate ctx.tasks while iterating over it"))?
            .push(eval.heap().alloc(configured));

        Ok(NoneType)
    }
}

/// Frozen task map — read-only after freeze.
#[derive(Debug, ProvidesStaticType, NoSerialize, Allocative)]
pub struct FrozenTaskMap {
    content: Vec<FrozenValue>,
}

impl fmt::Display for FrozenTaskMap {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "tasks")
    }
}

unsafe impl<'v> Trace<'v> for FrozenTaskMap {
    fn trace(&mut self, _tracer: &Tracer<'v>) {}
}

starlark_simple_value!(FrozenTaskMap);

#[starlark_value(type = "tasks")]
impl<'v> StarlarkValue<'v> for FrozenTaskMap {
    type Canonical = TaskMap<'v>;

    fn iterate_collect(&self, _heap: Heap<'v>) -> starlark::Result<Vec<Value<'v>>> {
        Ok(self.content.iter().map(|v| v.to_value()).collect())
    }

    fn at(&self, index: Value<'v>, _heap: Heap<'v>) -> starlark::Result<Value<'v>> {
        let key = index.unpack_str().ok_or_else(|| {
            starlark::Error::new_other(anyhow::anyhow!(
                "ctx.tasks key must be a string, got {}",
                index.get_type()
            ))
        })?;
        for fv in self.content.iter() {
            if let Some(ct) = fv.downcast_ref::<FrozenConfiguredTask>() {
                if task_key(&ct.group, &ct.name) == key {
                    return Ok(fv.to_value());
                }
            }
        }
        Err(starlark::Error::new_other(anyhow::anyhow!(
            "no task found at path {:?}",
            key
        )))
    }
}
