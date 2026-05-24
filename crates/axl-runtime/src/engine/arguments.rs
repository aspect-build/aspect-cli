use std::cell::RefCell;

use allocative::Allocative;
use derive_more::Display;
use starlark::collections::SmallMap;
use starlark::environment::Methods;
use starlark::environment::MethodsBuilder;
use starlark::environment::MethodsStatic;
use starlark::starlark_module;
use starlark::starlark_simple_value;
use starlark::values;
use starlark::values::Heap;
use starlark::values::NoSerialize;
use starlark::values::ProvidesStaticType;
use starlark::values::StarlarkValue;
use starlark::values::Trace;
use starlark::values::Tracer;
use starlark::values::Value;
use starlark::values::ValueLike;
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
///
/// `explicit_cli_keys` is a side-set tracking which keys came from clap's
/// `ValueSource::CommandLine` during the runtime-args merge. Exposed via
/// the `is_explicit(name)` Starlark method so repro/fix command builders
/// can skip echoing flags that match the task's default (including
/// alias-overridden defaults and config.axl overrides). Empty for the
/// config-time override store role — nothing is "explicit on the CLI"
/// there.
#[derive(Debug, Clone, ProvidesStaticType, Display, NoSerialize, Allocative)]
#[display("<Arguments>")]
pub struct Arguments<'v> {
    #[allocative(skip)]
    args: RefCell<SmallMap<String, Value<'v>>>,
    #[allocative(skip)]
    explicit_cli_keys: RefCell<SmallMap<String, ()>>,
}

impl<'v> Arguments<'v> {
    pub fn new() -> Self {
        Self {
            args: RefCell::new(SmallMap::new()),
            explicit_cli_keys: RefCell::new(SmallMap::new()),
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

    /// Mark `key` as having been supplied on the CLI for the current
    /// invocation. Idempotent. See `is_explicit_key`.
    pub fn mark_explicit(&self, key: String) {
        self.explicit_cli_keys.borrow_mut().insert(key, ());
    }

    /// Return `true` iff `key` was marked explicit during the runtime
    /// merge (i.e. clap saw it as `ValueSource::CommandLine`).
    pub fn is_explicit_key(&self, key: &str) -> bool {
        self.explicit_cli_keys.borrow().contains_key(key)
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

    fn get_methods() -> Option<&'static Methods> {
        static RES: MethodsStatic = MethodsStatic::new();
        RES.methods(arguments_methods)
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
        Ok(FrozenArguments {
            args: frozen,
            explicit_cli_keys: self.explicit_cli_keys.into_inner(),
        })
    }
}

#[derive(Debug, Display, ProvidesStaticType, NoSerialize, Allocative)]
#[display("<Arguments {args:?}>")]
pub struct FrozenArguments {
    #[allocative(skip)]
    args: SmallMap<String, values::FrozenValue>,
    #[allocative(skip)]
    explicit_cli_keys: SmallMap<String, ()>,
}

starlark_simple_value!(FrozenArguments);

impl FrozenArguments {
    pub fn get(&self, key: &str) -> Option<values::FrozenValue> {
        self.args.get(key).copied()
    }

    pub fn contains_key(&self, key: &str) -> bool {
        self.args.contains_key(key)
    }

    pub fn is_explicit_key(&self, key: &str) -> bool {
        self.explicit_cli_keys.contains_key(key)
    }
}

#[starlark_value(type = "Arguments")]
impl<'v> StarlarkValue<'v> for FrozenArguments {
    type Canonical = Arguments<'v>;

    fn get_attr(&self, key: &str, _heap: Heap<'v>) -> Option<Value<'v>> {
        self.args.get(key).map(|v| v.to_value())
    }

    fn get_methods() -> Option<&'static Methods> {
        static RES: MethodsStatic = MethodsStatic::new();
        RES.methods(arguments_methods)
    }
}

#[starlark_module]
fn arguments_methods(builder: &mut MethodsBuilder) {
    /// Return `True` iff the user passed `--<name>=...` (or its short
    /// form) on the CLI for the current invocation.
    ///
    /// Returns `False` for args that were resolved from the task's
    /// default, an alias's overridden default, or a `config.axl`
    /// override — anything that wasn't typed at the command line.
    ///
    /// Used by repro / fix command builders to skip echoing flags
    /// whose values would just reproduce the task's default. The
    /// developer copying the rendered repro gets back exactly what
    /// they (or CI) typed.
    fn is_explicit<'v>(
        this: Value<'v>,
        #[starlark(require = pos)] name: &str,
    ) -> anyhow::Result<bool> {
        if let Some(a) = this.downcast_ref::<Arguments>() {
            return Ok(a.is_explicit_key(name));
        }
        if let Some(a) = this.downcast_ref::<FrozenArguments>() {
            return Ok(a.is_explicit_key(name));
        }
        Err(anyhow::anyhow!(
            "is_explicit: expected an Arguments value, got '{}'",
            this.get_type(),
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn explicit_keys_default_to_unset() {
        let args = Arguments::new();
        assert!(!args.is_explicit_key("anything"));
        assert!(!args.is_explicit_key(""));
    }

    #[test]
    fn marked_keys_report_explicit() {
        let args = Arguments::new();
        args.mark_explicit("scope".to_string());
        args.mark_explicit("formatter_target".to_string());
        assert!(args.is_explicit_key("scope"));
        assert!(args.is_explicit_key("formatter_target"));
        assert!(!args.is_explicit_key("ignore_patterns"));
    }

    #[test]
    fn mark_explicit_is_idempotent() {
        let args = Arguments::new();
        args.mark_explicit("scope".to_string());
        args.mark_explicit("scope".to_string());
        assert!(args.is_explicit_key("scope"));
    }
}
