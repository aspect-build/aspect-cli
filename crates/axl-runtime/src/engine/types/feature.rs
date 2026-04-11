//! FeatureType and FeatureInstance — behavior-injection units for the fragment system.
//!
//! A feature is declared with `feature(implementation=fn, attrs={...})`, configured
//! by users in config.axl via `ctx.features[FeatureType].field = value`, and run
//! after all config.axl files have been evaluated. The `implementation` function
//! receives a `FeatureContext` and injects closures into fragment hook lists.
//!
//! Every feature instance has a built-in `enabled` field (defaults to `True`).
//! If the user sets `ctx.features[X].enabled = False`, the runtime skips calling
//! `implementation` for that feature.

use std::cell::Cell;
use std::fmt::{self, Display, Write};
use std::sync::atomic::{AtomicU64, Ordering};

use allocative::Allocative;
use dupe::Dupe;

use starlark::environment::{GlobalsBuilder, Methods, MethodsBuilder, MethodsStatic};
use starlark::starlark_module;
use starlark::values::dict::UnpackDictEntries;
use starlark::values::typing::TypeCompiled;
use starlark::values::{
    AllocFrozenValue, AllocValue, Freeze, FreezeError, Freezer, FrozenHeap, FrozenValue, Heap,
    NoSerialize, ProvidesStaticType, StarlarkValue, Trace, Tracer, Value, ValueLike,
    starlark_value,
};
use starlark_map::small_map::SmallMap;

use crate::engine::types::r#trait::{
    Field, FieldValue, FrozenField, FrozenFieldValue, build_type_checkers, copy_default_value,
};

static FEATURE_TYPE_ID: AtomicU64 = AtomicU64::new(0);

fn next_feature_type_id() -> u64 {
    FEATURE_TYPE_ID.fetch_add(1, Ordering::SeqCst)
}

// -----------------------------------------------------------------------------
// FeatureType
// -----------------------------------------------------------------------------

/// The type of a feature, created by `feature(implementation=fn, attrs={...})`.
/// Calling this type (internally) creates a `FeatureInstance` with default field values.
#[derive(Debug, ProvidesStaticType, NoSerialize, Allocative)]
pub struct FeatureType<'v> {
    /// Unique identifier for this feature type.
    pub(crate) id: u64,
    /// Name of the feature type (set when assigned to a variable).
    pub(crate) name: Option<String>,
    /// User-configurable fields declared via `attrs = {"field": attr(...)}`.
    pub(crate) fields: SmallMap<String, Field<'v>>,
    /// The injection function called after config.axl with a `FeatureContext`.
    #[allocative(skip)]
    pub(crate) implementation_fn: Option<Value<'v>>,
}

impl<'v> Display for FeatureType<'v> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match &self.name {
            Some(name) => write!(f, "feature[{}]", name),
            None => write!(f, "feature[anon]"),
        }
    }
}

unsafe impl<'v> Trace<'v> for FeatureType<'v> {
    fn trace(&mut self, tracer: &Tracer<'v>) {
        for (_, field) in self.fields.iter_mut() {
            field.trace(tracer);
        }
        if let Some(ref mut f) = self.implementation_fn {
            f.trace(tracer);
        }
    }
}

impl<'v> AllocValue<'v> for FeatureType<'v> {
    fn alloc_value(self, heap: Heap<'v>) -> Value<'v> {
        heap.alloc_complex(self)
    }
}

#[starlark_value(type = "feature")]
impl<'v> StarlarkValue<'v> for FeatureType<'v> {
    fn collect_repr(&self, collector: &mut String) {
        write!(collector, "{}", self).unwrap();
    }

    fn export_as(
        &self,
        variable_name: &str,
        _eval: &mut starlark::eval::Evaluator<'v, '_, '_>,
    ) -> starlark::Result<()> {
        let this = self as *const Self as *mut Self;
        unsafe {
            (*this).name = Some(variable_name.to_string());
        }
        Ok(())
    }

    fn invoke(
        &self,
        _me: Value<'v>,
        args: &starlark::eval::Arguments<'v, '_>,
        eval: &mut starlark::eval::Evaluator<'v, '_, '_>,
    ) -> starlark::Result<Value<'v>> {
        let type_checkers =
            build_type_checkers(self.fields.values().map(|f| f.typ_value), eval.heap())?;

        args.no_positional_args(eval.heap())?;
        let kwargs = args.names_map()?;

        let mut values: Vec<Cell<Value<'v>>> = Vec::with_capacity(self.fields.len());
        for ((field_name, field), tc) in self.fields.iter().zip(type_checkers.iter()) {
            let value = if let Some(v) = kwargs.get(field_name.as_str()) {
                *v
            } else if let Some(default) = field.default {
                copy_default_value(default, eval.heap())?
            } else {
                return Err(starlark::Error::new_other(anyhow::anyhow!(
                    "Missing required field `{}` for {}",
                    field_name,
                    self
                )));
            };

            if !tc.matches(value) {
                return Err(starlark::Error::new_other(anyhow::anyhow!(
                    "Field `{}` expected type `{}`, got `{}`",
                    field_name,
                    tc,
                    value.get_type()
                )));
            }

            values.push(Cell::new(value));
        }

        let instance = FeatureInstance {
            typ: _me,
            values: values.into_boxed_slice(),
            type_checkers: type_checkers.into_boxed_slice(),
            enabled: Cell::new(true),
        };
        Ok(eval.heap().alloc(instance))
    }

    fn get_methods() -> Option<&'static Methods> {
        static RES: MethodsStatic = MethodsStatic::new();
        RES.methods(feature_type_methods)
    }
}

#[starlark_module]
fn feature_type_methods(_builder: &mut MethodsBuilder) {}

// -----------------------------------------------------------------------------
// FrozenFeatureType
// -----------------------------------------------------------------------------

#[derive(Debug, ProvidesStaticType, NoSerialize, Allocative)]
pub struct FrozenFeatureType {
    pub(crate) id: u64,
    pub(crate) name: Option<String>,
    pub(crate) fields: SmallMap<String, FrozenField>,
    pub(crate) implementation_fn: Option<FrozenValue>,
}

impl Display for FrozenFeatureType {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match &self.name {
            Some(name) => write!(f, "feature[{}]", name),
            None => write!(f, "feature[anon]"),
        }
    }
}

unsafe impl<'v> Trace<'v> for FrozenFeatureType {
    fn trace(&mut self, _tracer: &Tracer<'v>) {}
}

impl AllocFrozenValue for FrozenFeatureType {
    fn alloc_frozen_value(self, heap: &FrozenHeap) -> FrozenValue {
        heap.alloc_simple(self)
    }
}

#[starlark_value(type = "feature")]
impl<'v> StarlarkValue<'v> for FrozenFeatureType {
    type Canonical = FeatureType<'v>;

    fn collect_repr(&self, collector: &mut String) {
        write!(collector, "{}", self).unwrap();
    }

    fn invoke(
        &self,
        _me: Value<'v>,
        args: &starlark::eval::Arguments<'v, '_>,
        eval: &mut starlark::eval::Evaluator<'v, '_, '_>,
    ) -> starlark::Result<Value<'v>> {
        let type_checkers = build_type_checkers(
            self.fields.values().map(|f| f.typ_value.to_value()),
            eval.heap(),
        )?;

        args.no_positional_args(eval.heap())?;
        let kwargs = args.names_map()?;

        let mut values: Vec<Cell<Value<'v>>> = Vec::with_capacity(self.fields.len());
        for ((field_name, field), tc) in self.fields.iter().zip(type_checkers.iter()) {
            let value = if let Some(v) = kwargs.get(field_name.as_str()) {
                *v
            } else if let Some(default) = field.default {
                copy_default_value(default.to_value(), eval.heap())?
            } else {
                return Err(starlark::Error::new_other(anyhow::anyhow!(
                    "Missing required field `{}` for {}",
                    field_name,
                    self
                )));
            };

            if !tc.matches(value) {
                return Err(starlark::Error::new_other(anyhow::anyhow!(
                    "Field `{}` expected type `{}`, got `{}`",
                    field_name,
                    tc,
                    value.get_type()
                )));
            }

            values.push(Cell::new(value));
        }

        let instance = FeatureInstance {
            typ: _me,
            values: values.into_boxed_slice(),
            type_checkers: type_checkers.into_boxed_slice(),
            enabled: Cell::new(true),
        };
        Ok(eval.heap().alloc(instance))
    }

    fn get_methods() -> Option<&'static Methods> {
        static RES: MethodsStatic = MethodsStatic::new();
        RES.methods(feature_type_methods)
    }
}

impl Freeze for FeatureType<'_> {
    type Frozen = FrozenFeatureType;

    fn freeze(self, freezer: &Freezer) -> Result<Self::Frozen, FreezeError> {
        let mut frozen_fields = SmallMap::with_capacity(self.fields.len());
        for (name, field) in self.fields.into_iter() {
            frozen_fields.insert(name, field.freeze(freezer)?);
        }
        Ok(FrozenFeatureType {
            id: self.id,
            name: self.name,
            fields: frozen_fields,
            implementation_fn: self
                .implementation_fn
                .map(|f| f.freeze(freezer))
                .transpose()?,
        })
    }
}

// -----------------------------------------------------------------------------
// FeatureInstance
// -----------------------------------------------------------------------------

/// An instance of a feature type, containing field values and the built-in `enabled` flag.
#[derive(Debug, ProvidesStaticType, NoSerialize, Allocative)]
pub struct FeatureInstance<'v> {
    /// The feature type this instance belongs to.
    pub(crate) typ: Value<'v>,
    /// Field values in the same order as the type's field definitions.
    #[allocative(skip)]
    pub(crate) values: Box<[Cell<Value<'v>>]>,
    /// Fresh type checkers created at construction time.
    #[allocative(skip)]
    pub(crate) type_checkers: Box<[TypeCompiled<Value<'v>>]>,
    /// Built-in enabled flag. Runtime skips `implementation` if false.
    #[allocative(skip)]
    pub(crate) enabled: Cell<bool>,
}

impl<'v> Display for FeatureInstance<'v> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}(enabled={}", self.typ, self.enabled.get())?;
        if let Some(feat_type) = self.typ.downcast_ref::<FeatureType>() {
            for ((name, _), value) in feat_type.fields.iter().zip(self.values.iter()) {
                write!(f, ", {}={}", name, value.get())?;
            }
        } else if let Some(frozen_type) = self.typ.downcast_ref::<FrozenFeatureType>() {
            for ((name, _), value) in frozen_type.fields.iter().zip(self.values.iter()) {
                write!(f, ", {}={}", name, value.get())?;
            }
        }
        write!(f, ")")
    }
}

unsafe impl<'v> Trace<'v> for FeatureInstance<'v> {
    fn trace(&mut self, tracer: &Tracer<'v>) {
        self.typ.trace(tracer);
        for cell in self.values.iter() {
            let mut v = cell.get();
            v.trace(tracer);
            cell.set(v);
        }
        for tc in self.type_checkers.iter_mut() {
            tc.trace(tracer);
        }
    }
}

impl<'v> AllocValue<'v> for FeatureInstance<'v> {
    fn alloc_value(self, heap: Heap<'v>) -> Value<'v> {
        heap.alloc_complex(self)
    }
}

#[starlark_value(type = "feature")]
impl<'v> StarlarkValue<'v> for FeatureInstance<'v> {
    fn collect_repr(&self, collector: &mut String) {
        write!(collector, "{}", self).unwrap();
    }

    fn get_attr(&self, attribute: &str, heap: Heap<'v>) -> Option<Value<'v>> {
        if attribute == "enabled" {
            return Some(heap.alloc(self.enabled.get()));
        }
        if let Some(feat_type) = self.typ.downcast_ref::<FeatureType>() {
            if let Some(idx) = feat_type.fields.get_index_of(attribute) {
                return Some(self.values[idx].get());
            }
        } else if let Some(frozen_type) = self.typ.downcast_ref::<FrozenFeatureType>() {
            if let Some(idx) = frozen_type.fields.get_index_of(attribute) {
                return Some(self.values[idx].get());
            }
        }
        None
    }

    fn set_attr(&self, attribute: &str, value: Value<'v>) -> starlark::Result<()> {
        if attribute == "enabled" {
            let b = value.unpack_bool().ok_or_else(|| {
                starlark::Error::new_other(anyhow::anyhow!(
                    "`enabled` must be a bool, got `{}`",
                    value.get_type()
                ))
            })?;
            self.enabled.set(b);
            return Ok(());
        }

        let idx = if let Some(feat_type) = self.typ.downcast_ref::<FeatureType>() {
            feat_type.fields.get_index_of(attribute)
        } else if let Some(frozen_type) = self.typ.downcast_ref::<FrozenFeatureType>() {
            frozen_type.fields.get_index_of(attribute)
        } else {
            return Err(starlark::Error::new_other(anyhow::anyhow!(
                "Invalid feature type"
            )));
        };

        let idx = match idx {
            Some(idx) => idx,
            None => {
                return Err(starlark::Error::new_other(anyhow::anyhow!(
                    "Feature {} has no field `{}`",
                    self.typ,
                    attribute
                )));
            }
        };

        let tc = &self.type_checkers[idx];
        if !tc.matches(value) {
            return Err(starlark::Error::new_other(anyhow::anyhow!(
                "Field `{}` expected type `{}`, got `{}`",
                attribute,
                tc,
                value.get_type()
            )));
        }

        self.values[idx].set(value);
        Ok(())
    }

    fn has_attr(&self, attribute: &str, _heap: Heap<'v>) -> bool {
        if attribute == "enabled" {
            return true;
        }
        if let Some(feat_type) = self.typ.downcast_ref::<FeatureType>() {
            feat_type.fields.contains_key(attribute)
        } else if let Some(frozen_type) = self.typ.downcast_ref::<FrozenFeatureType>() {
            frozen_type.fields.contains_key(attribute)
        } else {
            false
        }
    }

    fn dir_attr(&self) -> Vec<String> {
        let mut attrs = vec!["enabled".to_string()];
        if let Some(feat_type) = self.typ.downcast_ref::<FeatureType>() {
            attrs.extend(feat_type.fields.keys().map(|s| s.clone()));
        } else if let Some(frozen_type) = self.typ.downcast_ref::<FrozenFeatureType>() {
            attrs.extend(frozen_type.fields.keys().map(|s| s.clone()));
        }
        attrs
    }
}

// -----------------------------------------------------------------------------
// FrozenFeatureInstance
// -----------------------------------------------------------------------------

#[derive(Debug, ProvidesStaticType, NoSerialize, Allocative)]
pub struct FrozenFeatureInstance {
    pub(crate) typ: FrozenValue,
    pub(crate) values: Box<[FrozenValue]>,
    pub(crate) enabled: bool,
}

impl Display for FrozenFeatureInstance {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}(enabled={}", self.typ, self.enabled)?;
        if let Some(frozen_type) = self.typ.downcast_ref::<FrozenFeatureType>() {
            for ((name, _), value) in frozen_type.fields.iter().zip(self.values.iter()) {
                write!(f, ", {}={}", name, value)?;
            }
        }
        write!(f, ")")
    }
}

unsafe impl<'v> Trace<'v> for FrozenFeatureInstance {
    fn trace(&mut self, _tracer: &Tracer<'v>) {}
}

impl AllocFrozenValue for FrozenFeatureInstance {
    fn alloc_frozen_value(self, heap: &FrozenHeap) -> FrozenValue {
        heap.alloc_simple(self)
    }
}

#[starlark_value(type = "feature")]
impl<'v> StarlarkValue<'v> for FrozenFeatureInstance {
    type Canonical = FeatureInstance<'v>;

    fn collect_repr(&self, collector: &mut String) {
        write!(collector, "{}", self).unwrap();
    }

    fn get_attr(&self, attribute: &str, heap: Heap<'v>) -> Option<Value<'v>> {
        if attribute == "enabled" {
            return Some(heap.alloc(self.enabled));
        }
        if let Some(frozen_type) = self.typ.downcast_ref::<FrozenFeatureType>() {
            if let Some(idx) = frozen_type.fields.get_index_of(attribute) {
                return Some(self.values[idx].to_value());
            }
        }
        None
    }

    fn has_attr(&self, attribute: &str, _heap: Heap<'v>) -> bool {
        if attribute == "enabled" {
            return true;
        }
        if let Some(frozen_type) = self.typ.downcast_ref::<FrozenFeatureType>() {
            frozen_type.fields.contains_key(attribute)
        } else {
            false
        }
    }

    fn dir_attr(&self) -> Vec<String> {
        let mut attrs = vec!["enabled".to_string()];
        if let Some(frozen_type) = self.typ.downcast_ref::<FrozenFeatureType>() {
            attrs.extend(frozen_type.fields.keys().map(|s| s.clone()));
        }
        attrs
    }
}

impl Freeze for FeatureInstance<'_> {
    type Frozen = FrozenFeatureInstance;

    fn freeze(self, freezer: &Freezer) -> Result<Self::Frozen, FreezeError> {
        let typ = self.typ.freeze(freezer)?;
        let values: Result<Vec<_>, _> = self
            .values
            .iter()
            .map(|v| v.get().freeze(freezer))
            .collect();
        Ok(FrozenFeatureInstance {
            typ,
            values: values?.into_boxed_slice(),
            enabled: self.enabled.get(),
        })
    }
}

// -----------------------------------------------------------------------------
// Helpers
// -----------------------------------------------------------------------------

/// Extract the feature type ID from a Value that is a FeatureType or FrozenFeatureType.
pub fn extract_feature_type_id(value: Value) -> Option<u64> {
    if let Some(ft) = value.downcast_ref::<FeatureType>() {
        Some(ft.id)
    } else if let Some(ft) = value.downcast_ref::<FrozenFeatureType>() {
        Some(ft.id)
    } else {
        None
    }
}

/// Extract the implementation function from a FeatureType or FrozenFeatureType value.
pub fn extract_feature_impl_fn<'v>(value: Value<'v>) -> Option<Value<'v>> {
    if let Some(ft) = value.downcast_ref::<FeatureType>() {
        ft.implementation_fn
    } else if let Some(ft) = value.downcast_ref::<FrozenFeatureType>() {
        ft.implementation_fn.map(|f| f.to_value())
    } else {
        None
    }
}

// -----------------------------------------------------------------------------
// Global registration
// -----------------------------------------------------------------------------

#[starlark_module]
pub fn register_globals(globals: &mut GlobalsBuilder) {
    /// Declares a feature — a composable behavior injector for the fragment system.
    ///
    /// The `implementation` function receives a `FeatureContext` and runs after all
    /// config.axl files have been evaluated. It can inject closures into fragment
    /// hook lists via `ctx.fragments[FragmentType].hook.append(...)`.
    ///
    /// Every feature instance automatically has an `enabled` field (default `True`).
    /// Set `ctx.features[MyFeature].enabled = False` in config.axl to disable it.
    ///
    /// Example:
    /// ```starlark
    /// def _impl(ctx: FeatureContext):
    ///     bazel = ctx.traits[BazelTrait]
    ///     dry_run = ctx.attr.dry_run
    ///     def _build_end(ctx, state, exit_code):
    ///         print("build finished: exit_code=%d dry_run=%s" % (exit_code, dry_run))
    ///     bazel.build_end.append(_build_end)
    ///
    /// GithubStatusCheckBazelTask = feature(
    ///     implementation = _impl,
    ///     attrs = {
    ///         "dry_run": attr(bool, False),
    ///     }
    /// )
    /// ```
    fn feature<'v>(
        #[starlark(require = named)] implementation: Value<'v>,
        #[starlark(require = named, default = UnpackDictEntries::default())]
        attrs: UnpackDictEntries<String, Value<'v>>,
        eval: &mut starlark::eval::Evaluator<'v, '_, '_>,
    ) -> anyhow::Result<FeatureType<'v>> {
        let mut fields = SmallMap::with_capacity(attrs.entries.len());

        for (name, value) in attrs.entries.into_iter() {
            let field = if let Some(field_value) = value.downcast_ref::<FieldValue>() {
                Field {
                    typ: field_value.typ.dupe(),
                    typ_value: field_value.typ_value,
                    default: field_value.default,
                }
            } else if let Some(field_value) = value.downcast_ref::<FrozenFieldValue>() {
                let typ_value = field_value.typ_value.to_value();
                let typ = TypeCompiled::new(typ_value, eval.heap())
                    .map_err(|e| anyhow::anyhow!("{:?}", e))?;
                Field {
                    typ,
                    typ_value,
                    default: field_value.default.map(|v| v.to_value()),
                }
            } else {
                let typ = TypeCompiled::new(value, eval.heap())
                    .map_err(|e| anyhow::anyhow!("{:?}", e))?;
                Field {
                    typ,
                    typ_value: value,
                    default: None,
                }
            };
            fields.insert(name, field);
        }

        Ok(FeatureType {
            id: next_feature_type_id(),
            name: None,
            fields,
            implementation_fn: Some(implementation),
        })
    }
}
