//! FragmentMap - A Starlark value that maps fragment type IDs to instances.

use std::cell::RefCell;
use std::fmt::{self, Display, Write};

use allocative::Allocative;
use starlark::starlark_simple_value;
use starlark::values::{
    AllocValue, Freeze, FreezeError, Freezer, FrozenValue, Heap, NoSerialize, ProvidesStaticType,
    StarlarkValue, Trace, Tracer, Value, ValueLike, starlark_value,
};
use starlark_map::small_map::SmallMap;

use crate::engine::types::fragment::{FragmentType, FrozenFragmentType, extract_fragment_type_id};

/// A Starlark value that maps fragment type IDs to their instances.
///
/// Used as `ctx.fragments` in both ConfigContext and TaskContext.
/// Supports `ctx.fragments[FragType]` for reading and
/// `ctx.fragments[FragType] = FragType(...)` for writing.
#[derive(Debug, ProvidesStaticType, NoSerialize, Allocative)]
pub struct FragmentMap<'v> {
    /// Map from fragment type id → (type_value, instance_value)
    #[allocative(skip)]
    entries: RefCell<SmallMap<u64, (Value<'v>, Value<'v>)>>,
}

impl<'v> Display for FragmentMap<'v> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "FragmentMap([")?;
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

unsafe impl<'v> Trace<'v> for FragmentMap<'v> {
    fn trace(&mut self, tracer: &Tracer<'v>) {
        let entries = self.entries.get_mut();
        for (_, (type_val, instance_val)) in entries.iter_mut() {
            type_val.trace(tracer);
            instance_val.trace(tracer);
        }
    }
}

impl<'v> AllocValue<'v> for FragmentMap<'v> {
    fn alloc_value(self, heap: &'v Heap) -> Value<'v> {
        heap.alloc_complex(self)
    }
}

impl<'v> Freeze for FragmentMap<'v> {
    type Frozen = FrozenFragmentMap;

    fn freeze(self, freezer: &Freezer) -> Result<Self::Frozen, FreezeError> {
        let entries = self.entries.into_inner();
        let mut frozen_entries = SmallMap::with_capacity(entries.len());
        for (id, (type_val, instance_val)) in entries.into_iter() {
            frozen_entries.insert(
                id,
                (type_val.freeze(freezer)?, instance_val.freeze(freezer)?),
            );
        }
        Ok(FrozenFragmentMap {
            entries: frozen_entries,
        })
    }
}

impl<'v> FragmentMap<'v> {
    /// Create a new empty FragmentMap.
    pub fn new() -> Self {
        FragmentMap {
            entries: RefCell::new(SmallMap::new()),
        }
    }

    /// Insert a fragment type and its default instance.
    pub fn insert(&self, type_id: u64, type_value: Value<'v>, instance: Value<'v>) {
        self.entries
            .borrow_mut()
            .insert(type_id, (type_value, instance));
    }

    /// Check if a fragment type is already present.
    pub fn contains(&self, type_id: u64) -> bool {
        self.entries.borrow().contains_key(&type_id)
    }

    /// Get instance for a given type ID.
    pub fn get_instance(&self, type_id: u64) -> Option<Value<'v>> {
        self.entries.borrow().get(&type_id).map(|(_, v)| *v)
    }

    /// Get all entries as (type_id, type_value, instance_value) tuples.
    pub fn entries(&self) -> Vec<(u64, Value<'v>, Value<'v>)> {
        self.entries
            .borrow()
            .iter()
            .map(|(id, (tv, iv))| (*id, *tv, *iv))
            .collect()
    }

    /// Create a new FragmentMap containing only the given type IDs,
    /// copying instance references from this map.
    pub fn scoped(&self, type_ids: &[u64], heap: &'v Heap) -> Value<'v> {
        let scoped = FragmentMap::new();
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

#[starlark_value(type = "FragmentMap")]
impl<'v> StarlarkValue<'v> for FragmentMap<'v> {
    fn collect_repr(&self, collector: &mut String) {
        write!(collector, "{}", self).unwrap();
    }

    fn at(&self, index: Value<'v>, _heap: &'v Heap) -> starlark::Result<Value<'v>> {
        let type_id = extract_fragment_type_id(index).ok_or_else(|| {
            starlark::Error::new_other(anyhow::anyhow!(
                "FragmentMap key must be a fragment type, got '{}'",
                index.get_type()
            ))
        })?;

        let entries = self.entries.borrow();
        match entries.get(&type_id) {
            Some((_, instance)) => Ok(*instance),
            None => {
                let type_name = if let Some(ft) = index.downcast_ref::<FragmentType>() {
                    ft.name.as_deref().unwrap_or("anon")
                } else if let Some(ft) = index.downcast_ref::<FrozenFragmentType>() {
                    ft.name.as_deref().unwrap_or("anon")
                } else {
                    "unknown"
                };
                Err(starlark::Error::new_other(anyhow::anyhow!(
                    "Fragment type '{}' not found in FragmentMap. Is it declared in a task's fragments list?",
                    type_name
                )))
            }
        }
    }

    fn set_at(&self, index: Value<'v>, new_value: Value<'v>) -> starlark::Result<()> {
        let type_id = extract_fragment_type_id(index).ok_or_else(|| {
            starlark::Error::new_other(anyhow::anyhow!(
                "FragmentMap key must be a fragment type, got '{}'",
                index.get_type()
            ))
        })?;

        let mut entries = self.entries.borrow_mut();
        match entries.get_mut(&type_id) {
            Some(entry) => {
                entry.1 = new_value;
                Ok(())
            }
            None => {
                // Auto-insert if not already present
                entries.insert(type_id, (index, new_value));
                Ok(())
            }
        }
    }
}

/// Frozen version of FragmentMap. Read-only after freezing.
#[derive(Debug, ProvidesStaticType, NoSerialize, Allocative)]
pub struct FrozenFragmentMap {
    #[allocative(skip)]
    entries: SmallMap<u64, (FrozenValue, FrozenValue)>,
}

impl Display for FrozenFragmentMap {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "FragmentMap([")?;
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

unsafe impl<'v> Trace<'v> for FrozenFragmentMap {
    fn trace(&mut self, _tracer: &Tracer<'v>) {
        // Frozen values don't need tracing
    }
}

starlark_simple_value!(FrozenFragmentMap);

#[starlark_value(type = "FragmentMap")]
impl<'v> StarlarkValue<'v> for FrozenFragmentMap {
    type Canonical = FragmentMap<'v>;

    fn collect_repr(&self, collector: &mut String) {
        write!(collector, "{}", self).unwrap();
    }

    fn at(&self, index: Value<'v>, _heap: &'v Heap) -> starlark::Result<Value<'v>> {
        let type_id = extract_fragment_type_id(index).ok_or_else(|| {
            starlark::Error::new_other(anyhow::anyhow!(
                "FragmentMap key must be a fragment type, got '{}'",
                index.get_type()
            ))
        })?;

        match self.entries.get(&type_id) {
            Some((_, instance)) => Ok(instance.to_value()),
            None => {
                let type_name = if let Some(ft) = index.downcast_ref::<FragmentType>() {
                    ft.name.as_deref().unwrap_or("anon")
                } else if let Some(ft) = index.downcast_ref::<FrozenFragmentType>() {
                    ft.name.as_deref().unwrap_or("anon")
                } else {
                    "unknown"
                };
                Err(starlark::Error::new_other(anyhow::anyhow!(
                    "Fragment type '{}' not found in FragmentMap. Is it declared in a task's fragments list?",
                    type_name
                )))
            }
        }
    }
}

/// Auto-construct fragment instances by calling each fragment type with no arguments
/// (using defaults from attr() definitions).
pub fn construct_fragments<'v>(
    fragment_types: &[(u64, Value<'v>)],
    eval: &mut starlark::eval::Evaluator<'v, '_, '_>,
    _heap: &'v Heap,
) -> Result<FragmentMap<'v>, crate::eval::EvalError> {
    let map = FragmentMap::new();
    for (type_id, type_value) in fragment_types {
        if !map.contains(*type_id) {
            let instance = eval.eval_function(*type_value, &[], &[]).map_err(|e| {
                crate::eval::EvalError::UnknownError(anyhow::anyhow!(
                    "Failed to construct default fragment instance for {}: {:?}",
                    type_value,
                    e
                ))
            })?;
            map.insert(*type_id, *type_value, instance);
        }
    }
    Ok(map)
}
