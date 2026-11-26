//! TraitMap - A Starlark value that maps trait type IDs to instances.

use std::cell::RefCell;
use std::fmt::{self, Display, Write};

use allocative::Allocative;
use starlark::starlark_simple_value;
use starlark::values::{
    AllocValue, Freeze, FreezeError, Freezer, FrozenValue, Heap, NoSerialize, ProvidesStaticType,
    StarlarkValue, Trace, Tracer, Value, ValueLike, starlark_value,
};
use starlark_map::small_map::SmallMap;

use super::r#trait::{
    FrozenTraitType, TraitType, construct_default_instance, extract_trait_type_id,
};

/// A Starlark value that maps trait type IDs to their instances.
///
/// One map serves a whole run: it is `ctx.traits` in ConfigContext,
/// FeatureContext and TaskContext alike, so what a config writes and the
/// hooks a feature registers are what the task reads. Supports
/// `ctx.traits[TraitType]` for reading and
/// `ctx.traits[TraitType] = TraitType(...)` for writing, for any trait type.
///
/// Instances are created lazily: the first `at` (read) of a type constructs
/// its default instance and caches it for subsequent calls. `insert` stores a
/// type without an instance, so [`TraitMap::with_fields`] can find a type
/// nothing has read yet.
#[derive(Debug, ProvidesStaticType, NoSerialize, Allocative)]
pub struct TraitMap<'v> {
    /// Map from trait type id → (type_value, Option<instance_value>)
    /// The instance is None until first accessed.
    #[allocative(skip)]
    entries: RefCell<SmallMap<u64, (Value<'v>, Option<Value<'v>>)>>,
}

impl<'v> Display for TraitMap<'v> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "TraitMap([")?;
        let entries = self.entries.borrow();
        let mut first = true;
        for (_, (type_val, _)) in entries.iter() {
            if !first {
                write!(f, ", ")?;
            }
            first = false;
            write!(f, "{}", type_val)?;
        }
        write!(f, "])")
    }
}

unsafe impl<'v> Trace<'v> for TraitMap<'v> {
    fn trace(&mut self, tracer: &Tracer<'v>) {
        let entries = self.entries.get_mut();
        for (_, (type_val, instance_val)) in entries.iter_mut() {
            type_val.trace(tracer);
            if let Some(iv) = instance_val {
                iv.trace(tracer);
            }
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
            // Only freeze entries that were actually instantiated.
            if let Some(iv) = instance_val {
                frozen_entries.insert(id, (type_val.freeze(freezer)?, iv.freeze(freezer)?));
            }
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

    /// Register a trait type without instantiating it. The default instance
    /// is created on first access; registering is only needed for a type
    /// [`TraitMap::with_fields`] must be able to find before anything reads it.
    pub fn insert(&self, type_id: u64, type_value: Value<'v>) {
        self.entries
            .borrow_mut()
            .entry(type_id)
            .or_insert((type_value, None));
    }

    /// Get the instance for a type ID if it has already been instantiated.
    pub fn get_instance(&self, type_id: u64) -> Option<Value<'v>> {
        self.entries.borrow().get(&type_id).and_then(|(_, v)| *v)
    }

    /// Get all instantiated entries as (type_id, type_value, instance_value) tuples.
    pub fn entries(&self) -> Vec<(u64, Value<'v>, Value<'v>)> {
        self.entries
            .borrow()
            .iter()
            .filter_map(|(id, (tv, iv))| iv.map(|i| (*id, *tv, i)))
            .collect()
    }

    /// The instance of a trait type with every field in `fields`, created
    /// now if it was not yet; None when no type in the map has them all. An
    /// instantiated match wins over one only registered with `insert`. For
    /// the runtime, which knows a builtin trait only by its shape: a type's
    /// name is not always recorded.
    pub fn with_fields(&self, fields: &[&str], heap: Heap<'v>) -> Option<Value<'v>> {
        let matches = |tv: &Value<'v>| {
            fields.iter().all(|f| {
                tv.downcast_ref::<TraitType>()
                    .map(|t| t.fields.contains_key(*f))
                    .or_else(|| {
                        tv.downcast_ref::<FrozenTraitType>()
                            .map(|t| t.fields.contains_key(*f))
                    })
                    .unwrap_or(false)
            })
        };
        let type_value = {
            let entries = self.entries.borrow();
            entries
                .values()
                .find(|(tv, iv)| iv.is_some() && matches(tv))
                .or_else(|| entries.values().find(|(tv, _)| matches(tv)))
                .map(|(tv, _)| *tv)
        }?;
        self.at(type_value, heap).ok()
    }
}

#[starlark_value(type = "TraitMap")]
impl<'v> StarlarkValue<'v> for TraitMap<'v> {
    fn collect_repr(&self, collector: &mut String) {
        write!(collector, "{}", self).unwrap();
    }

    fn at(&self, index: Value<'v>, heap: Heap<'v>) -> starlark::Result<Value<'v>> {
        let type_id = extract_trait_type_id(index).ok_or_else(|| {
            starlark::Error::new_other(anyhow::anyhow!(
                "TraitMap key must be a trait type, got '{}'",
                index.get_type()
            ))
        })?;

        if let Some((_, Some(instance))) = self.entries.borrow().get(&type_id) {
            return Ok(*instance);
        }
        let instance = construct_default_instance(index, heap)?;
        self.entries
            .borrow_mut()
            .entry(type_id)
            .or_insert((index, None))
            .1 = Some(instance);
        Ok(instance)
    }

    /// `Trait in ctx.traits` is True for every trait type: any type reads
    /// as its instance, created on first access. A non-trait key is absent,
    /// not an error.
    fn is_in(&self, other: Value<'v>) -> starlark::Result<bool> {
        Ok(extract_trait_type_id(other).is_some())
    }

    fn set_at(&self, index: Value<'v>, new_value: Value<'v>) -> starlark::Result<()> {
        let type_id = extract_trait_type_id(index).ok_or_else(|| {
            starlark::Error::new_other(anyhow::anyhow!(
                "TraitMap key must be a trait type, got '{}'",
                index.get_type()
            ))
        })?;

        self.entries
            .borrow_mut()
            .entry(type_id)
            .or_insert((index, None))
            .1 = Some(new_value);
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

impl FrozenTraitMap {
    /// Get all entries as (type_id, type_value, instance_value) tuples.
    pub fn entries(&self) -> Vec<(u64, Value<'_>, Value<'_>)> {
        self.entries
            .iter()
            .map(|(id, (tv, iv))| (*id, tv.to_value(), iv.to_value()))
            .collect()
    }
}

impl Display for FrozenTraitMap {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "TraitMap([")?;
        let mut first = true;
        for (_, (type_val, _)) in self.entries.iter() {
            if !first {
                write!(f, ", ")?;
            }
            first = false;
            write!(f, "{}", type_val)?;
        }
        write!(f, "])")
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
        let type_id = extract_trait_type_id(index).ok_or_else(|| {
            starlark::Error::new_other(anyhow::anyhow!(
                "TraitMap key must be a trait type, got '{}'",
                index.get_type()
            ))
        })?;

        match self.entries.get(&type_id) {
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
