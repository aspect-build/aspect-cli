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

use crate::engine::types::r#trait::{
    FrozenTraitType, TraitType, construct_default_instance, extract_trait_type_id,
};

/// A Starlark value that maps trait type IDs to their instances.
///
/// Used as `ctx.traits` in both ConfigContext and TaskContext.
/// Supports `ctx.traits[TraitType]` for reading and
/// `ctx.traits[TraitType] = TraitType(...)` for writing.
///
/// Instances are created lazily: registering a trait type with `insert` stores
/// only the type value; the default instance is constructed on the first `at`
/// (read) access and cached for subsequent calls.
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

    /// Register a trait type. The default instance is created lazily on first access.
    pub fn insert(&self, type_id: u64, type_value: Value<'v>) {
        self.entries
            .borrow_mut()
            .entry(type_id)
            .or_insert((type_value, None));
    }

    /// Check if a trait type is registered (whether or not it has been instantiated).
    pub fn contains(&self, type_id: u64) -> bool {
        self.entries.borrow().contains_key(&type_id)
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

    /// Create a new TraitMap containing only the given type IDs.
    /// Both instantiated and uninstantiated entries are copied so that the
    /// scoped map can still lazily construct instances on first access.
    pub fn scoped(&self, type_ids: &[u64], heap: Heap<'v>) -> Value<'v> {
        let scoped = TraitMap::new();
        let entries = self.entries.borrow();
        for id in type_ids {
            if let Some((type_val, instance_val)) = entries.get(id) {
                scoped
                    .entries
                    .borrow_mut()
                    .insert(*id, (*type_val, *instance_val));
            }
        }
        heap.alloc(scoped)
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

        // Fast path: already instantiated.
        {
            let entries = self.entries.borrow();
            match entries.get(&type_id) {
                Some((_, Some(instance))) => return Ok(*instance),
                None => {
                    let type_name = if let Some(ft) = index.downcast_ref::<TraitType>() {
                        ft.name.as_deref().unwrap_or("anon")
                    } else if let Some(ft) = index.downcast_ref::<FrozenTraitType>() {
                        ft.name.as_deref().unwrap_or("anon")
                    } else {
                        "unknown"
                    };
                    return Err(starlark::Error::new_other(anyhow::anyhow!(
                        "Trait type '{}' not found in TraitMap. Is it declared in a task's traits list?",
                        type_name
                    )));
                }
                Some((_, None)) => {} // fall through to lazy construction
            }
        }

        // Lazy construction: build the default instance and cache it.
        let type_val = self
            .entries
            .borrow()
            .get(&type_id)
            .map(|(tv, _)| *tv)
            .unwrap();
        let instance = construct_default_instance(type_val, heap)?;
        self.entries.borrow_mut().get_mut(&type_id).unwrap().1 = Some(instance);
        Ok(instance)
    }

    fn set_at(&self, index: Value<'v>, new_value: Value<'v>) -> starlark::Result<()> {
        let type_id = extract_trait_type_id(index).ok_or_else(|| {
            starlark::Error::new_other(anyhow::anyhow!(
                "TraitMap key must be a trait type, got '{}'",
                index.get_type()
            ))
        })?;

        let mut entries = self.entries.borrow_mut();
        match entries.get_mut(&type_id) {
            Some(entry) => {
                entry.1 = Some(new_value);
                Ok(())
            }
            None => {
                entries.insert(type_id, (index, Some(new_value)));
                Ok(())
            }
        }
    }
}

/// Frozen version of TraitMap. Read-only after freezing.
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

    fn at(&self, index: Value<'v>, _heap: Heap<'v>) -> starlark::Result<Value<'v>> {
        let type_id = extract_trait_type_id(index).ok_or_else(|| {
            starlark::Error::new_other(anyhow::anyhow!(
                "TraitMap key must be a trait type, got '{}'",
                index.get_type()
            ))
        })?;

        match self.entries.get(&type_id) {
            Some((_, instance)) => Ok(instance.to_value()),
            None => {
                let type_name = if let Some(ft) = index.downcast_ref::<TraitType>() {
                    ft.name.as_deref().unwrap_or("anon")
                } else if let Some(ft) = index.downcast_ref::<FrozenTraitType>() {
                    ft.name.as_deref().unwrap_or("anon")
                } else {
                    "unknown"
                };
                Err(starlark::Error::new_other(anyhow::anyhow!(
                    "Trait type '{}' not found in TraitMap. Is it declared in a task's traits list?",
                    type_name
                )))
            }
        }
    }
}
