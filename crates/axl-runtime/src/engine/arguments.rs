use std::cell::RefCell;

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
use starlark::values::Tracer;
use starlark::values::Value;
use starlark::values::list::AllocList;
use starlark::values::starlark_value;

/// A typed bag of `name -> Value` pairs, exposed to Starlark with attribute access.
///
/// Used in two roles:
///
/// - **Runtime args** — `ctx.args` inside a task or feature implementation. Built once
///   from CLI parse results + config.axl overrides, then frozen.
/// - **Config-time override store** — `ctx.tasks["k"].args` and `ctx.features[X].args`
///   in `config.axl`. Mutable via `set_attr`; presence of a key marks it as
///   "explicitly set in config.axl" for runtime precedence (CLI > config > default).
#[derive(Debug, Clone, ProvidesStaticType, Display, NoSerialize, Allocative)]
#[display("<Arguments>")]
pub struct Arguments<'v> {
    #[allocative(skip)]
    args: RefCell<SmallMap<String, Value<'v>>>,
}

impl<'v> Arguments<'v> {
    pub fn new() -> Self {
        Self {
            args: RefCell::new(SmallMap::new()),
        }
    }

    pub fn insert(&self, key: String, value: Value<'v>) {
        self.args.borrow_mut().insert(key, value);
    }

    pub fn contains_key(&self, key: &str) -> bool {
        self.args.borrow().contains_key(key)
    }

    pub fn get(&self, key: &str) -> Option<Value<'v>> {
        self.args.borrow().get(key).cloned()
    }

    /// Snapshot of the current `(name, value)` pairs.
    pub fn entries(&self) -> Vec<(String, Value<'v>)> {
        self.args
            .borrow()
            .iter()
            .map(|(k, v)| (k.clone(), *v))
            .collect()
    }

    pub fn alloc_list<L>(items: L) -> AllocList<L> {
        AllocList(items)
    }
}

unsafe impl<'v> Trace<'v> for Arguments<'v> {
    fn trace(&mut self, tracer: &Tracer<'v>) {
        for (_, v) in self.args.get_mut().iter_mut() {
            v.trace(tracer);
        }
    }
}

#[starlark_value(type = "Arguments")]
impl<'v> StarlarkValue<'v> for Arguments<'v> {
    fn get_attr(&self, key: &str, _heap: Heap<'v>) -> Option<Value<'v>> {
        self.args.borrow().get(key).cloned()
    }

    fn set_attr(&self, attribute: &str, value: Value<'v>) -> starlark::Result<()> {
        self.args.borrow_mut().insert(attribute.to_owned(), value);
        Ok(())
    }
}

impl<'v> values::AllocValue<'v> for Arguments<'v> {
    fn alloc_value(self, heap: Heap<'v>) -> Value<'v> {
        heap.alloc_complex(self)
    }
}

impl<'v> values::Freeze for Arguments<'v> {
    type Frozen = FrozenArguments;
    fn freeze(self, freezer: &values::Freezer) -> values::FreezeResult<Self::Frozen> {
        let inner = self.args.into_inner();
        let mut frozen = SmallMap::with_capacity(inner.len());
        for (k, v) in inner.into_iter() {
            frozen.insert(k, v.freeze(freezer)?);
        }
        Ok(FrozenArguments { args: frozen })
    }
}

#[derive(Debug, Display, ProvidesStaticType, NoSerialize, Allocative)]
#[display("<Arguments {args:?}>")]
pub struct FrozenArguments {
    #[allocative(skip)]
    args: SmallMap<String, values::FrozenValue>,
}

starlark_simple_value!(FrozenArguments);

impl FrozenArguments {
    pub fn get(&self, key: &str) -> Option<values::FrozenValue> {
        self.args.get(key).copied()
    }

    pub fn contains_key(&self, key: &str) -> bool {
        self.args.contains_key(key)
    }
}

#[starlark_value(type = "Arguments")]
impl<'v> StarlarkValue<'v> for FrozenArguments {
    type Canonical = Arguments<'v>;

    fn get_attr(&self, key: &str, _heap: Heap<'v>) -> Option<Value<'v>> {
        self.args.get(key).map(|v| v.to_value())
    }
}
