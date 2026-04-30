//! TaskMap — a collection of `Task` values, indexed by canonical `group/name` key.
//!
//! Used as `ctx.tasks` in ConfigContext (mutable) and exposed for iteration.
//! Supports `ctx.tasks["group/name"]` for read access and `ctx.tasks.add(task)`
//! for adding tasks dynamically from `config.axl`. The values themselves carry
//! their own interior mutability for config.axl overrides.

use std::cell::RefCell;
use std::fmt::{self, Display};

use allocative::Allocative;
use anyhow::anyhow;
use starlark::environment::{Methods, MethodsBuilder, MethodsStatic};
use starlark::eval::Evaluator;
use starlark::starlark_module;
use starlark::starlark_simple_value;
use starlark::values::{
    AllocValue, Freeze, FreezeError, Freezer, FrozenValue, Heap, NoSerialize, ProvidesStaticType,
    StarlarkValue, Trace, Tracer, Value, ValueLike, none::NoneType, starlark_value,
};

use super::task::{FrozenTask, Task};

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

fn task_key_of(value: Value<'_>) -> Option<String> {
    if let Some(t) = value.downcast_ref::<Task>() {
        Some(task_key(t.group(), &t.name()))
    } else {
        value
            .downcast_ref::<FrozenTask>()
            .map(|t| task_key(&t.group, &t.name))
    }
}

#[derive(Debug, ProvidesStaticType, NoSerialize, Allocative)]
pub struct TaskMap<'v> {
    #[allocative(skip)]
    entries: RefCell<Vec<Value<'v>>>,
}

impl<'v> TaskMap<'v> {
    pub fn new() -> Self {
        TaskMap {
            entries: RefCell::new(Vec::new()),
        }
    }

    pub fn from_values(values: Vec<Value<'v>>) -> Self {
        TaskMap {
            entries: RefCell::new(values),
        }
    }

    pub fn insert(&self, task: Value<'v>) {
        self.entries.borrow_mut().push(task);
    }

    pub fn values(&self) -> Vec<Value<'v>> {
        self.entries.borrow().clone()
    }
}

impl<'v> Display for TaskMap<'v> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "tasks")
    }
}

unsafe impl<'v> Trace<'v> for TaskMap<'v> {
    fn trace(&mut self, tracer: &Tracer<'v>) {
        for v in self.entries.get_mut().iter_mut() {
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

    fn freeze(self, freezer: &Freezer) -> Result<Self::Frozen, FreezeError> {
        let entries = self
            .entries
            .into_inner()
            .into_iter()
            .map(|v| v.freeze(freezer))
            .collect::<Result<Vec<_>, _>>()?;
        Ok(FrozenTaskMap { entries })
    }
}

#[starlark_value(type = "tasks")]
impl<'v> StarlarkValue<'v> for TaskMap<'v> {
    fn iterate_collect(&self, _heap: Heap<'v>) -> starlark::Result<Vec<Value<'v>>> {
        Ok(self.entries.borrow().clone())
    }

    fn at(&self, index: Value<'v>, _heap: Heap<'v>) -> starlark::Result<Value<'v>> {
        let key = index.unpack_str().ok_or_else(|| {
            starlark::Error::new_other(anyhow!(
                "ctx.tasks key must be a string, got {}",
                index.get_type()
            ))
        })?;
        for v in self.entries.borrow().iter() {
            if task_key_of(*v).as_deref() == Some(key) {
                return Ok(*v);
            }
        }
        Err(starlark::Error::new_other(anyhow!(
            "no task found at path {:?}",
            key
        )))
    }

    fn get_methods() -> Option<&'static Methods> {
        static RES: MethodsStatic = MethodsStatic::new();
        RES.methods(task_map_methods)
    }
}

#[starlark_module]
fn task_map_methods(registry: &mut MethodsBuilder) {
    /// Add a task to the task map.
    ///
    /// The task value must be a frozen task (i.e. defined at module level, not inside a
    /// function). After adding, the task is accessible via iteration and by key:
    /// `ctx.tasks["group/name"].args.x = val`.
    fn add<'v>(
        this: Value<'v>,
        #[starlark(require = pos)] task: Value<'v>,
        eval: &mut Evaluator<'v, '_, '_>,
    ) -> anyhow::Result<NoneType> {
        // Imported tasks (`load(...)` from another module) come in as `FrozenTask`.
        // Thaw them onto the shared heap so the map only ever holds live `Task` values
        // and config.axl can mutate their args via the override store.
        let task_value: Value<'v> = if task.downcast_ref::<Task>().is_some() {
            task
        } else if task.downcast_ref::<FrozenTask>().is_some() {
            let frozen_value = task
                .unpack_frozen()
                .expect("FrozenTask value is always frozen");
            eval.heap()
                .alloc(Task::from_frozen(frozen_value, eval.heap()))
        } else {
            return Err(anyhow!(
                "ctx.tasks.add: expected a task() value, got '{}'",
                task.get_type()
            ));
        };

        let key = task_key_of(task_value)
            .ok_or_else(|| anyhow!("ctx.tasks.add: task value missing name/group"))?;
        if key.is_empty() {
            return Err(anyhow!("ctx.tasks.add: task must have a non-empty name"));
        }

        let map = this
            .downcast_ref::<TaskMap>()
            .ok_or_else(|| anyhow!("ctx.tasks is not a task map"))?;
        {
            let content = map
                .entries
                .try_borrow()
                .map_err(|_| anyhow!("cannot read ctx.tasks while mutating it"))?;
            if content
                .iter()
                .any(|v| task_key_of(*v).as_deref() == Some(&key))
            {
                return Err(anyhow!("ctx.tasks.add: task {:?} already exists", key));
            }
        }
        map.entries
            .try_borrow_mut()
            .map_err(|_| anyhow!("cannot mutate ctx.tasks while iterating over it"))?
            .push(task_value);
        Ok(NoneType)
    }
}

#[derive(Debug, ProvidesStaticType, NoSerialize, Allocative)]
pub struct FrozenTaskMap {
    #[allocative(skip)]
    entries: Vec<FrozenValue>,
}

impl FrozenTaskMap {
    pub fn values(&self) -> Vec<Value<'_>> {
        self.entries.iter().map(|fv| fv.to_value()).collect()
    }
}

impl Display for FrozenTaskMap {
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
        Ok(self.entries.iter().map(|fv| fv.to_value()).collect())
    }

    fn at(&self, index: Value<'v>, _heap: Heap<'v>) -> starlark::Result<Value<'v>> {
        let key = index.unpack_str().ok_or_else(|| {
            starlark::Error::new_other(anyhow!(
                "ctx.tasks key must be a string, got {}",
                index.get_type()
            ))
        })?;
        for fv in self.entries.iter() {
            if task_key_of(fv.to_value()).as_deref() == Some(key) {
                return Ok(fv.to_value());
            }
        }
        Err(starlark::Error::new_other(anyhow!(
            "no task found at path {:?}",
            key
        )))
    }
}
