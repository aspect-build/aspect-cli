use allocative::Allocative;
use derive_more::Display;
use starlark::collections::SmallMap;
use starlark::starlark_simple_value;

use starlark::values;
use starlark::values::Heap;
use starlark::values::NoSerialize;
use starlark::values::ProvidesStaticType;
use starlark::values::StarlarkValue;
use starlark::values::Trace;
use starlark::values::Value;
use starlark::values::list::AllocList;
use starlark::values::starlark_value;

#[derive(Debug, Clone, ProvidesStaticType, Display, Trace, NoSerialize, Allocative)]
#[display("<task_args>")]
pub struct TaskArgs<'v> {
    #[allocative(skip)]
    args: SmallMap<String, values::Value<'v>>,
}

impl<'v> TaskArgs<'v> {
    pub fn new() -> Self {
        Self {
            args: SmallMap::new(),
        }
    }

    #[inline]
    pub fn insert(&mut self, key: String, value: values::Value<'v>) {
        self.args.insert(key, value);
    }

    #[inline]
    pub fn alloc_list<L>(items: L) -> AllocList<L> {
        AllocList(items)
    }
}

#[starlark_value(type = "task_args")]
impl<'v> StarlarkValue<'v> for TaskArgs<'v> {
    fn get_attr(&self, key: &str, _heap: &'v Heap) -> Option<Value<'v>> {
        self.args.get(key).cloned()
    }
}

impl<'v> values::AllocValue<'v> for TaskArgs<'v> {
    fn alloc_value(self, heap: &'v values::Heap) -> Value<'v> {
        heap.alloc_complex(self)
    }
}

impl<'v> values::Freeze for TaskArgs<'v> {
    type Frozen = FrozenTaskArgs;
    fn freeze(self, freezer: &values::Freezer) -> values::FreezeResult<Self::Frozen> {
        Ok(FrozenTaskArgs {
            args: self
                .args
                .iter()
                .map(|(k, v)| (k.clone(), v.freeze(freezer).unwrap()))
                .collect(),
        })
    }
}

#[derive(Debug, Display, ProvidesStaticType, NoSerialize, Allocative)]
#[display("<task_args {args:?}>")]
pub struct FrozenTaskArgs {
    #[allocative(skip)]
    args: SmallMap<String, values::FrozenValue>,
}

starlark_simple_value!(FrozenTaskArgs);

#[starlark_value(type = "task_args")]
impl<'v> StarlarkValue<'v> for FrozenTaskArgs {
    type Canonical = TaskArgs<'v>;
}
