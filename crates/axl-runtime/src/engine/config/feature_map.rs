//! FeatureMap - A Starlark value that maps feature type IDs to instances.

use std::cell::RefCell;
use std::fmt::{self, Display, Write};

use allocative::Allocative;
use starlark::starlark_simple_value;
use starlark::values::{
    AllocValue, Freeze, FreezeError, Freezer, FrozenValue, Heap, NoSerialize, ProvidesStaticType,
    StarlarkValue, Trace, Tracer, Value, ValueLike, starlark_value,
};
use starlark_map::small_map::SmallMap;

use crate::engine::types::feature::{FeatureType, FrozenFeatureType, extract_feature_type_id};

/// A Starlark value that maps feature type IDs to their instances.
///
/// Used as `ctx.features` in ConfigContext (mutable) and TaskContext (frozen).
/// Supports `ctx.features[FeatureType]` for reading and
/// `ctx.features[FeatureType].field = value` for writing (via get + set_attr on instance).
#[derive(Debug, ProvidesStaticType, NoSerialize, Allocative)]
pub struct FeatureMap<'v> {
    #[allocative(skip)]
    entries: RefCell<SmallMap<u64, (Value<'v>, Value<'v>)>>,
}

impl<'v> Display for FeatureMap<'v> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "FeatureMap([")?;
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

unsafe impl<'v> Trace<'v> for FeatureMap<'v> {
    fn trace(&mut self, tracer: &Tracer<'v>) {
        let entries = self.entries.get_mut();
        for (_, (type_val, instance_val)) in entries.iter_mut() {
            type_val.trace(tracer);
            instance_val.trace(tracer);
        }
    }
}

impl<'v> AllocValue<'v> for FeatureMap<'v> {
    fn alloc_value(self, heap: Heap<'v>) -> Value<'v> {
        heap.alloc_complex(self)
    }
}

impl<'v> Freeze for FeatureMap<'v> {
    type Frozen = FrozenFeatureMap;

    fn freeze(self, freezer: &Freezer) -> Result<Self::Frozen, FreezeError> {
        let entries = self.entries.into_inner();
        let mut frozen_entries = SmallMap::with_capacity(entries.len());
        for (id, (type_val, instance_val)) in entries.into_iter() {
            // Freeze the type descriptor but not the instance. Keeping instances
            // unfrozen allows mutable `state = {}` dicts captured in feature
            // closures to remain writable during task execution.
            //
            // SAFETY: Instance values live on the config heap, which is kept
            // alive for the full duration of task execution.
            let instance_static: Value<'static> =
                unsafe { std::mem::transmute(instance_val) };
            frozen_entries.insert(id, (type_val.freeze(freezer)?, instance_static));
        }
        Ok(FrozenFeatureMap {
            entries: frozen_entries,
        })
    }
}

impl<'v> FeatureMap<'v> {
    pub fn new() -> Self {
        FeatureMap {
            entries: RefCell::new(SmallMap::new()),
        }
    }

    pub fn insert(&self, type_id: u64, type_value: Value<'v>, instance: Value<'v>) {
        self.entries
            .borrow_mut()
            .insert(type_id, (type_value, instance));
    }

    pub fn contains(&self, type_id: u64) -> bool {
        self.entries.borrow().contains_key(&type_id)
    }

    pub fn entries(&self) -> Vec<(u64, Value<'v>, Value<'v>)> {
        self.entries
            .borrow()
            .iter()
            .map(|(id, (tv, iv))| (*id, *tv, *iv))
            .collect()
    }
}

#[starlark_value(type = "FeatureMap")]
impl<'v> StarlarkValue<'v> for FeatureMap<'v> {
    fn collect_repr(&self, collector: &mut String) {
        write!(collector, "{}", self).unwrap();
    }

    fn at(&self, index: Value<'v>, _heap: Heap<'v>) -> starlark::Result<Value<'v>> {
        let type_id = extract_feature_type_id(index).ok_or_else(|| {
            starlark::Error::new_other(anyhow::anyhow!(
                "FeatureMap key must be a feature type, got '{}'",
                index.get_type()
            ))
        })?;

        let entries = self.entries.borrow();
        match entries.get(&type_id) {
            Some((_, instance)) => Ok(*instance),
            None => {
                let type_name = if let Some(ft) = index.downcast_ref::<FeatureType>() {
                    ft.name.as_deref().unwrap_or("anon")
                } else if let Some(ft) = index.downcast_ref::<FrozenFeatureType>() {
                    ft.name.as_deref().unwrap_or("anon")
                } else {
                    "unknown"
                };
                Err(starlark::Error::new_other(anyhow::anyhow!(
                    "Feature type '{}' not found in FeatureMap. Is it declared via use_feature() in MODULE.aspect?",
                    type_name
                )))
            }
        }
    }
}

/// Frozen version of FeatureMap. Type values are frozen; instance values retain
/// mutability so that mutable `state = {}` dicts in feature closures remain
/// writable during task execution.
#[derive(Debug, ProvidesStaticType, NoSerialize, Allocative)]
pub struct FrozenFeatureMap {
    #[allocative(skip)]
    entries: SmallMap<u64, (FrozenValue, Value<'static>)>,
}

// SAFETY: AXL runtime is single-threaded; these values are never accessed concurrently.
unsafe impl Send for FrozenFeatureMap {}
unsafe impl Sync for FrozenFeatureMap {}

impl FrozenFeatureMap {
    pub fn entries(&self) -> Vec<(u64, Value<'_>, Value<'_>)> {
        self.entries
            .iter()
            .map(|(id, (tv, iv))| {
                // SAFETY: instance values are alive for the duration of task execution.
                let iv: Value<'_> = unsafe { std::mem::transmute(*iv) };
                (*id, tv.to_value(), iv)
            })
            .collect()
    }
}

impl Display for FrozenFeatureMap {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "FeatureMap([")?;
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

unsafe impl<'v> Trace<'v> for FrozenFeatureMap {
    fn trace(&mut self, _tracer: &Tracer<'v>) {}
}

starlark_simple_value!(FrozenFeatureMap);

#[starlark_value(type = "FeatureMap")]
impl<'v> StarlarkValue<'v> for FrozenFeatureMap {
    type Canonical = FeatureMap<'v>;

    fn collect_repr(&self, collector: &mut String) {
        write!(collector, "{}", self).unwrap();
    }

    fn at(&self, index: Value<'v>, _heap: Heap<'v>) -> starlark::Result<Value<'v>> {
        let type_id = extract_feature_type_id(index).ok_or_else(|| {
            starlark::Error::new_other(anyhow::anyhow!(
                "FeatureMap key must be a feature type, got '{}'",
                index.get_type()
            ))
        })?;

        match self.entries.get(&type_id) {
            // SAFETY: instance values are alive for the duration of task execution.
            Some((_, instance)) => Ok(unsafe { std::mem::transmute(*instance) }),
            None => {
                let type_name = if let Some(ft) = index.downcast_ref::<FeatureType>() {
                    ft.name.as_deref().unwrap_or("anon")
                } else if let Some(ft) = index.downcast_ref::<FrozenFeatureType>() {
                    ft.name.as_deref().unwrap_or("anon")
                } else {
                    "unknown"
                };
                Err(starlark::Error::new_other(anyhow::anyhow!(
                    "Feature type '{}' not found in FeatureMap.",
                    type_name
                )))
            }
        }
    }
}

/// Construct default feature instances for each feature type.
/// Called before config.axl evaluation, after fragment construction.
pub fn construct_features<'v>(
    feature_types: &[(u64, Value<'v>)],
    eval: &mut starlark::eval::Evaluator<'v, '_, '_>,
) -> Result<FeatureMap<'v>, crate::eval::EvalError> {
    let map = FeatureMap::new();
    for (type_id, type_value) in feature_types {
        if !map.contains(*type_id) {
            let instance = eval.eval_function(*type_value, &[], &[]).map_err(|e| {
                crate::eval::EvalError::UnknownError(anyhow::anyhow!(
                    "Failed to construct default feature instance for {}: {:?}",
                    type_value,
                    e
                ))
            })?;
            map.insert(*type_id, *type_value, instance);
        }
    }
    Ok(map)
}
