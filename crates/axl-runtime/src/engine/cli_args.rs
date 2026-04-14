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
#[display("<CliArgs>")]
pub struct CliArgs<'v> {
    #[allocative(skip)]
    args: SmallMap<String, values::Value<'v>>,
}

impl<'v> CliArgs<'v> {
    pub fn new() -> Self {
        Self {
            args: SmallMap::new(),
        }
    }

    /// Creates CliArgs from a HashMap of string key-value pairs, allocating strings on the heap.
    pub fn from_map(map: std::collections::HashMap<String, String>, heap: Heap<'v>) -> Self {
        let mut args = SmallMap::new();
        for (key, value) in map {
            args.insert(key, heap.alloc_str(&value).to_value());
        }
        Self { args }
    }

    #[inline]
    pub fn insert(&mut self, key: String, value: values::Value<'v>) {
        self.args.insert(key, value);
    }

    #[inline]
    pub fn contains_key(&self, key: &str) -> bool {
        self.args.contains_key(key)
    }

    #[inline]
    pub fn get(&self, key: &str) -> Option<Value<'v>> {
        self.args.get(key).cloned()
    }

    #[inline]
    pub fn alloc_list<L>(items: L) -> AllocList<L> {
        AllocList(items)
    }
}

#[starlark_value(type = "CliArgs")]
impl<'v> StarlarkValue<'v> for CliArgs<'v> {
    fn get_attr(&self, key: &str, _heap: Heap<'v>) -> Option<Value<'v>> {
        self.args.get(key).cloned()
    }
}

impl<'v> values::AllocValue<'v> for CliArgs<'v> {
    fn alloc_value(self, heap: values::Heap<'v>) -> Value<'v> {
        heap.alloc_complex(self)
    }
}

impl<'v> values::Freeze for CliArgs<'v> {
    type Frozen = FrozenCliArgs;
    fn freeze(self, freezer: &values::Freezer) -> values::FreezeResult<Self::Frozen> {
        Ok(FrozenCliArgs {
            args: self
                .args
                .iter()
                .map(|(k, v)| (k.clone(), v.freeze(freezer).unwrap()))
                .collect(),
        })
    }
}

#[derive(Debug, Display, ProvidesStaticType, NoSerialize, Allocative)]
#[display("<CliArgs {args:?}>")]
pub struct FrozenCliArgs {
    #[allocative(skip)]
    args: SmallMap<String, values::FrozenValue>,
}

starlark_simple_value!(FrozenCliArgs);

#[starlark_value(type = "CliArgs")]
impl<'v> StarlarkValue<'v> for FrozenCliArgs {
    type Canonical = CliArgs<'v>;

    fn get_attr(&self, key: &str, _heap: Heap<'v>) -> Option<Value<'v>> {
        self.args.get(key).cloned().map(|x| x.to_value())
    }
}
