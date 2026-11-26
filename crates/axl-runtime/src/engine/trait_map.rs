//! TraitMap - A Starlark value that maps trait type IDs to instances.

use std::cell::RefCell;
use std::fmt::{self, Display, Write};

use allocative::Allocative;
use starlark::starlark_simple_value;
use starlark::values::{
    AllocValue, Freeze, FreezeError, Freezer, FrozenValue, Heap, NoSerialize, ProvidesStaticType,
    StarlarkValue, Trace, Tracer, Value, starlark_value,
};
use starlark_map::small_map::SmallMap;

use super::r#trait::{construct_default_instance, extract_trait_type_id};

/// A Starlark value that maps trait type IDs to their instances.
///
/// One map serves a whole run: it is `ctx.traits` in ConfigContext,
/// FeatureContext and TaskContext alike, so what a config writes and the
/// hooks a feature registers are what the task reads. Supports
/// `ctx.traits[TraitType]` for reading and
/// `ctx.traits[TraitType] = TraitType(...)` for writing, for any trait type.
///
/// Instances are created lazily: the first `at` (read) of a type constructs
/// its default instance and caches it for subsequent calls.
#[derive(Debug, ProvidesStaticType, NoSerialize, Allocative)]
pub struct TraitMap<'v> {
    /// Map from trait type id to (type_value, instance_value).
    #[allocative(skip)]
    entries: RefCell<SmallMap<u64, (Value<'v>, Value<'v>)>>,
}

impl<'v> Display for TraitMap<'v> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write_types(f, self.entries.borrow().values().map(|(tv, _)| *tv))
    }
}

/// `TraitMap([A, B])`, naming the types a map holds instances of.
fn write_types<'v>(
    f: &mut fmt::Formatter<'_>,
    types: impl Iterator<Item = Value<'v>>,
) -> fmt::Result {
    write!(f, "TraitMap([")?;
    for (i, type_val) in types.enumerate() {
        if i > 0 {
            write!(f, ", ")?;
        }
        write!(f, "{}", type_val)?;
    }
    write!(f, "])")
}

unsafe impl<'v> Trace<'v> for TraitMap<'v> {
    fn trace(&mut self, tracer: &Tracer<'v>) {
        for (_, (type_val, instance_val)) in self.entries.get_mut().iter_mut() {
            type_val.trace(tracer);
            instance_val.trace(tracer);
        }
    }
}

impl<'v> AllocValue<'v> for TraitMap<'v> {
    fn alloc_value(self, heap: Heap<'v>) -> Value<'v> {
        heap.alloc_complex(self)
    }
}

impl<'v> Freeze for TraitMap<'v> {
    type Frozen = FrozenTraitMap;

    fn freeze(self, freezer: &Freezer) -> Result<Self::Frozen, FreezeError> {
        let entries = self.entries.into_inner();
        let mut frozen_entries = SmallMap::with_capacity(entries.len());
        for (id, (type_val, instance_val)) in entries.into_iter() {
            frozen_entries.insert(
                id,
                (type_val.freeze(freezer)?, instance_val.freeze(freezer)?),
            );
        }
        Ok(FrozenTraitMap {
            entries: frozen_entries,
        })
    }
}

impl<'v> TraitMap<'v> {
    /// Create a new empty TraitMap.
    pub fn new() -> Self {
        TraitMap {
            entries: RefCell::new(SmallMap::new()),
        }
    }

    /// Every instance created so far, in creation order.
    pub fn instances(&self) -> Vec<Value<'v>> {
        self.entries.borrow().values().map(|(_, iv)| *iv).collect()
    }
}

/// The trait type id of a `ctx.traits[...]` key, or the error for a key
/// that is not a trait type.
fn key_id(index: Value) -> starlark::Result<u64> {
    extract_trait_type_id(index).ok_or_else(|| {
        starlark::Error::new_other(anyhow::anyhow!(
            "TraitMap key must be a trait type, got '{}'",
            index.get_type()
        ))
    })
}

#[starlark_value(type = "TraitMap")]
impl<'v> StarlarkValue<'v> for TraitMap<'v> {
    fn collect_repr(&self, collector: &mut String) {
        write!(collector, "{}", self).unwrap();
    }

    fn at(&self, index: Value<'v>, heap: Heap<'v>) -> starlark::Result<Value<'v>> {
        let type_id = key_id(index)?;
        if let Some((_, instance)) = self.entries.borrow().get(&type_id) {
            return Ok(*instance);
        }
        let instance = construct_default_instance(index, heap)?;
        self.entries.borrow_mut().insert(type_id, (index, instance));
        Ok(instance)
    }

    /// `Trait in ctx.traits` is True for every trait type: any type reads
    /// as its instance, created on first access. A non-trait key is absent,
    /// not an error.
    fn is_in(&self, other: Value<'v>) -> starlark::Result<bool> {
        Ok(extract_trait_type_id(other).is_some())
    }

    fn set_at(&self, index: Value<'v>, new_value: Value<'v>) -> starlark::Result<()> {
        let type_id = key_id(index)?;
        self.entries
            .borrow_mut()
            .insert(type_id, (index, new_value));
        Ok(())
    }
}

/// Frozen version of TraitMap: a snapshot of the instances a live map had
/// created. Read-only.
///
/// Only a frozen `TaskContext` holds one, and a running task never sees
/// its context frozen: tasks run on the run's live heap, which is not
/// frozen. A type the snapshot has no instance for reads as a fresh
/// default instance on the reader's heap, which is not kept, matching
/// what the live map would have created.
#[derive(Debug, ProvidesStaticType, NoSerialize, Allocative)]
pub struct FrozenTraitMap {
    #[allocative(skip)]
    entries: SmallMap<u64, (FrozenValue, FrozenValue)>,
}

impl Display for FrozenTraitMap {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write_types(f, self.entries.values().map(|(tv, _)| tv.to_value()))
    }
}

unsafe impl<'v> Trace<'v> for FrozenTraitMap {
    fn trace(&mut self, _tracer: &Tracer<'v>) {
        // Frozen values don't need tracing
    }
}

starlark_simple_value!(FrozenTraitMap);

#[starlark_value(type = "TraitMap")]
impl<'v> StarlarkValue<'v> for FrozenTraitMap {
    type Canonical = TraitMap<'v>;

    fn collect_repr(&self, collector: &mut String) {
        write!(collector, "{}", self).unwrap();
    }

    fn at(&self, index: Value<'v>, heap: Heap<'v>) -> starlark::Result<Value<'v>> {
        match self.entries.get(&key_id(index)?) {
            Some((_, instance)) => Ok(instance.to_value()),
            None => construct_default_instance(index, heap),
        }
    }

    /// True for every trait type, as for the live map.
    fn is_in(&self, other: Value<'v>) -> starlark::Result<bool> {
        Ok(extract_trait_type_id(other).is_some())
    }
}

#[cfg(test)]
mod tests {
    //! One trait map serves config, features and the task, and any trait
    //! type can be read from it without being declared anywhere.

    /// A task reads a trait no one declared and sees what the config wrote
    /// and the hook a feature registered; `in` is True for any trait type.
    #[test]
    fn a_task_sees_config_writes_and_feature_hooks_on_any_trait() {
        let exit = crate::test::eval(
            r#"
MyTrait = trait(
    greeting = attr(str, default = "default"),
    hooks = attr(list, default = []),
)
Untouched = trait(n = attr(int, default = 7))

def _feature_impl(ctx):
    ctx.traits[MyTrait].hooks.append(lambda: "from feature")

Hooks = feature(implementation = _feature_impl)

def _impl(ctx):
    t = ctx.traits[MyTrait]
    if t.greeting != "from config": return 1
    if [h() for h in t.hooks] != ["from feature"]: return 2
    if Untouched not in ctx.traits: return 3
    if ctx.traits[Untouched].n != 7: return 4
    if "x" in ctx.traits: return 5
    return 0

probe = task(implementation = _impl)
"#,
        )
        .with_features(&["Hooks"])
        .with_config(
            r#"
load("./test.axl", "MyTrait")

def config(ctx):
    ctx.traits[MyTrait].greeting = "from config"
"#,
        )
        .run_task(0)
        .expect("run_task");
        assert_eq!(exit, Some(0));
    }

    /// A config can write a trait no task or feature references.
    #[test]
    fn a_config_writes_a_trait_nothing_else_references() {
        let exit = crate::test::eval(
            r#"
Lonely = trait(n = attr(int, default = 0))

def _impl(ctx):
    return 0

probe = task(implementation = _impl)
"#,
        )
        .with_config(
            r#"
load("./test.axl", "Lonely")

def config(ctx):
    if Lonely not in ctx.traits: fail("not in")
    if ctx.traits[Lonely].n != 0: fail("not the default")
    ctx.traits[Lonely].n = 3
    ctx.traits[Lonely] = Lonely(n = 4)
"#,
        )
        .run_task(0)
        .expect("run_task");
        assert_eq!(exit, Some(0));
    }
}
