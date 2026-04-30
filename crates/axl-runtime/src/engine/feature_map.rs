//! FeatureMap — a collection of `Feature` values.
//!
//! Used as `ctx.features` in ConfigContext (mutable) and FeatureContext (frozen).
//! Supports `ctx.features[X]` for read access where `X` is a feature value;
//! the lookup matches by Starlark pointer equality (the same allocation flowing
//! through `load(...)` reaches both the map and the index expression). The
//! values themselves carry their own interior mutability for config.axl
//! overrides — the map just holds them.

use std::cell::RefCell;
use std::fmt::{self, Display, Write};

use allocative::Allocative;
use starlark::starlark_simple_value;
use starlark::values::{
    AllocValue, Freeze, FreezeError, Freezer, FrozenValue, Heap, NoSerialize, ProvidesStaticType,
    StarlarkValue, Trace, Tracer, Value, ValueLike, starlark_value,
};

use super::feature::{Feature, FrozenFeature};

/// Returns the `FrozenValue` handle that identifies a feature across heaps.
///
/// The user's `ctx.features[ArtifactUpload]` index expression resolves to a
/// `FrozenValue<FrozenFeature>` from the loaded `features.axl` module, while
/// the map holds *live* `Feature` values thawed onto the shared heap.
/// Both sides agree on the same `FrozenValue` (the frozen def). Live Features
/// remember theirs in `Feature::frozen_handle`; loaded Features are the handle.
fn handle_of(value: Value<'_>) -> Option<FrozenValue> {
    if let Some(live) = value.downcast_ref::<Feature>() {
        return live.frozen_handle();
    }
    if value.downcast_ref::<FrozenFeature>().is_some() {
        return value.unpack_frozen();
    }
    None
}

#[derive(Debug, ProvidesStaticType, NoSerialize, Allocative)]
pub struct FeatureMap<'v> {
    #[allocative(skip)]
    entries: RefCell<Vec<Value<'v>>>,
}

impl<'v> FeatureMap<'v> {
    pub fn new() -> Self {
        FeatureMap {
            entries: RefCell::new(Vec::new()),
        }
    }

    pub fn insert(&self, feature: Value<'v>) {
        self.entries.borrow_mut().push(feature);
    }

    pub fn values(&self) -> Vec<Value<'v>> {
        self.entries.borrow().clone()
    }
}

impl<'v> Display for FeatureMap<'v> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "FeatureMap([")?;
        let entries = self.entries.borrow();
        for (i, v) in entries.iter().enumerate() {
            if i > 0 {
                write!(f, ", ")?;
            }
            write!(f, "{}", v)?;
        }
        write!(f, "])")
    }
}

unsafe impl<'v> Trace<'v> for FeatureMap<'v> {
    fn trace(&mut self, tracer: &Tracer<'v>) {
        for v in self.entries.get_mut().iter_mut() {
            v.trace(tracer);
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
        let entries = self
            .entries
            .into_inner()
            .into_iter()
            .map(|v| v.freeze(freezer))
            .collect::<Result<Vec<_>, _>>()?;
        Ok(FrozenFeatureMap { entries })
    }
}

#[starlark_value(type = "FeatureMap")]
impl<'v> StarlarkValue<'v> for FeatureMap<'v> {
    fn collect_repr(&self, collector: &mut String) {
        write!(collector, "{}", self).unwrap();
    }

    fn at(&self, index: Value<'v>, _heap: Heap<'v>) -> starlark::Result<Value<'v>> {
        let handle = handle_of(index).ok_or_else(|| {
            starlark::Error::new_other(anyhow::anyhow!(
                "FeatureMap key must be a feature, got '{}'",
                index.get_type()
            ))
        })?;
        for v in self.entries.borrow().iter() {
            if handle_of(*v) == Some(handle) {
                return Ok(*v);
            }
        }
        Err(starlark::Error::new_other(anyhow::anyhow!(
            "Feature '{}' not found in FeatureMap. Is it declared via use_feature() in MODULE.aspect?",
            index
        )))
    }
}

#[derive(Debug, ProvidesStaticType, NoSerialize, Allocative)]
pub struct FrozenFeatureMap {
    #[allocative(skip)]
    entries: Vec<FrozenValue>,
}

impl FrozenFeatureMap {
    pub fn values(&self) -> Vec<Value<'_>> {
        self.entries.iter().map(|fv| fv.to_value()).collect()
    }
}

impl Display for FrozenFeatureMap {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "FeatureMap([")?;
        for (i, v) in self.entries.iter().enumerate() {
            if i > 0 {
                write!(f, ", ")?;
            }
            write!(f, "{}", v)?;
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
        let handle = handle_of(index).ok_or_else(|| {
            starlark::Error::new_other(anyhow::anyhow!(
                "FeatureMap key must be a feature, got '{}'",
                index.get_type()
            ))
        })?;
        for fv in self.entries.iter() {
            if handle_of(fv.to_value()) == Some(handle) {
                return Ok(fv.to_value());
            }
        }
        Err(starlark::Error::new_other(anyhow::anyhow!(
            "Feature '{}' not found in FeatureMap.",
            index
        )))
    }
}
