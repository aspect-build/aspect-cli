//! FeatureType and FeatureInstance — behavior-injection units for the fragment system.
//!
//! A feature is declared with `feature(implementation=fn, args={...})`, configured
//! by users in config.axl via `ctx.features[FeatureType].field = value`, and run
//! after all config.axl files have been evaluated. The `implementation` function
//! receives a `FeatureContext` and injects closures into fragment hook lists.
//!
//! Every feature instance has a built-in `enabled` field (defaults to `True`).
//! If the user sets `ctx.features[X].enabled = False`, the runtime skips calling
//! `implementation` for that feature.

use std::cell::{Cell, RefCell};
use std::fmt::{self, Display, Write};
use std::sync::atomic::{AtomicU64, Ordering};

use allocative::Allocative;

use starlark::environment::{GlobalsBuilder, Methods, MethodsBuilder, MethodsStatic};
use starlark::starlark_module;
use starlark::values::dict::UnpackDictEntries;
use starlark::values::list::AllocList;
use starlark::values::typing::TypeCompiled;
use starlark::values::{
    AllocFrozenValue, AllocValue, Freeze, FreezeError, Freezer, FrozenHeap, FrozenValue, Heap,
    NoSerialize, OwnedFrozenValue, ProvidesStaticType, StarlarkValue, Trace, Tracer, Value,
    ValueError, ValueLike, starlark_value,
};
use starlark_map::small_map::SmallMap;

use crate::engine::arg::Arg;
use crate::engine::cli_args::CliArgs;
use crate::engine::config::freeze_value;
pub use crate::engine::types::names::{
    camel_to_display_name, to_command_name, to_display_name, validate_arg_name,
    validate_command_name, validate_type_name,
};

static FEATURE_TYPE_ID: AtomicU64 = AtomicU64::new(0);

fn next_feature_type_id() -> u64 {
    FEATURE_TYPE_ID.fetch_add(1, Ordering::SeqCst)
}

// -----------------------------------------------------------------------------
// FeatureType
// -----------------------------------------------------------------------------

/// The type of a feature, created by `feature(implementation=fn, args={...})`.
/// Calling this type (internally) creates a `FeatureInstance` with default field values.
#[derive(Debug, ProvidesStaticType, NoSerialize, Allocative)]
pub struct FeatureType<'v> {
    /// Unique identifier for this feature type.
    pub(crate) id: u64,
    /// Kebab-case slug used as the CLI arg prefix, e.g. `"artifact-upload"`.
    /// Set by the `name` parameter in `feature()`, or auto-derived from `export_name`
    /// via `to_command_name` when the value is assigned to a module-level variable.
    /// Empty until `export_as` fires when no explicit `name` was given.
    pub(crate) name: String,
    /// CamelCase Starlark variable name, set when the value is assigned to a module-level
    /// variable via `export_as` (e.g. `"ArtifactUpload"`). `None` for anonymous features.
    pub(crate) export_name: Option<String>,
    /// Human-readable display name, e.g. `"Artifact Upload"`. Derived from `export_name`
    /// if not set explicitly.
    pub(crate) display_name: String,
    /// One-line summary shown in the task list. Empty means use the "defined in" fallback.
    pub(crate) summary: String,
    /// Extended description shown in `--help` after the summary. Empty means omit.
    pub(crate) description: String,
    /// Unified arg map. CLI-exposed entries are shown in help; `Custom` entries are
    /// config.axl-only and not shown in help.
    pub(crate) args: SmallMap<String, Arg>,
    /// The injection function called after config.axl with a `FeatureContext`.
    #[allocative(skip)]
    pub(crate) implementation_fn: Option<Value<'v>>,
}

impl<'v> Display for FeatureType<'v> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if let Some(ref n) = self.export_name {
            write!(f, "feature[{}]", n)
        } else if !self.name.is_empty() {
            write!(f, "feature[{}]", self.name)
        } else {
            write!(f, "feature[anon]")
        }
    }
}

unsafe impl<'v> Trace<'v> for FeatureType<'v> {
    fn trace(&mut self, tracer: &Tracer<'v>) {
        // Arg has no Value<'v> fields; nothing to trace there.
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
        validate_type_name(variable_name, "feature")
            .map_err(|e| starlark::Error::new_other(anyhow::anyhow!(e)))?;
        let this = self as *const Self as *mut Self;
        unsafe {
            (*this).export_name = Some(variable_name.to_string());
            // Derive the kebab slug from the export name when no explicit name was given.
            if (&(*this).name).is_empty() {
                (*this).name = to_command_name(variable_name);
            }
            // Bake the fully-qualified `--<feature-name>:enabled` long flag into the implicit
            // `enabled` arg. This guarantees the correct unique Clap ID at registration time,
            // bypassing any downstream prefix-extraction logic that might return an empty string.
            let feature_name = (*this).name.clone();
            if !feature_name.is_empty() {
                if let Some(enabled_arg) = (*this).args.get_mut("enabled") {
                    if let Arg::Boolean { long, .. } = enabled_arg {
                        *long = Some(format!("{}:enabled", feature_name));
                    }
                }
            }
        }
        Ok(())
    }

    fn invoke(
        &self,
        _me: Value<'v>,
        args: &starlark::eval::Arguments<'v, '_>,
        eval: &mut starlark::eval::Evaluator<'v, '_, '_>,
    ) -> starlark::Result<Value<'v>> {
        // Only Custom args are stored in the FeatureInstance; CLI args come from the CLI.
        let custom_args: Vec<(&str, Option<FrozenValue>, Option<FrozenValue>)> = self
            .args
            .iter()
            .filter_map(|(k, v)| match v {
                Arg::Custom {
                    typ_value, default, ..
                } => Some((k.as_str(), *typ_value, *default)),
                _ => None,
            })
            .collect();

        // Build type checkers where possible; None when typ_value wasn't frozen.
        let type_checkers: Vec<Option<TypeCompiled<Value<'v>>>> = custom_args
            .iter()
            .map(|(_, typ_value, _)| {
                typ_value
                    .map(|fv| TypeCompiled::new(fv.to_value(), eval.heap()))
                    .transpose()
                    .map_err(|e| starlark::Error::new_other(anyhow::anyhow!("{:?}", e)))
            })
            .collect::<starlark::Result<_>>()?;

        args.no_positional_args(eval.heap())?;
        let kwargs = args.names_map()?;

        let mut values: Vec<Cell<Value<'v>>> = Vec::with_capacity(custom_args.len());
        for ((field_name, _, default), tc) in custom_args.iter().zip(type_checkers.iter()) {
            let value = if let Some(v) = kwargs.get(*field_name) {
                *v
            } else if let Some(default_fv) = default {
                default_fv.to_value()
            } else {
                // No storable default (e.g. lambda default was dropped at definition time).
                // Fall back to None so the arg is accessible as ctx.args.name.
                eval.heap().alloc(starlark::values::none::NoneType)
            };

            if let Some(tc) = tc {
                if !tc.matches(value) {
                    return Err(starlark::Error::new_other(anyhow::anyhow!(
                        "Arg `{}` expected type `{}`, got `{}`",
                        field_name,
                        tc,
                        value.get_type()
                    )));
                }
            }

            values.push(Cell::new(value));
        }

        let instance = FeatureInstance {
            typ: _me,
            values: values.into_boxed_slice(),
            type_checkers: type_checkers.into_boxed_slice(),
            config_overrides: RefCell::new(SmallMap::new()),
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
    pub(crate) name: String,
    pub(crate) export_name: Option<String>,
    pub(crate) display_name: String,
    pub(crate) summary: String,
    pub(crate) description: String,
    pub(crate) args: SmallMap<String, Arg>,
    pub(crate) implementation_fn: Option<FrozenValue>,
}

impl Display for FrozenFeatureType {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if let Some(ref n) = self.export_name {
            write!(f, "feature[{}]", n)
        } else if !self.name.is_empty() {
            write!(f, "feature[{}]", self.name)
        } else {
            write!(f, "feature[anon]")
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
        // Only Custom args are stored in the FeatureInstance.
        let custom_args: Vec<(&str, Option<FrozenValue>, Option<FrozenValue>)> = self
            .args
            .iter()
            .filter_map(|(k, v)| match v {
                Arg::Custom {
                    typ_value, default, ..
                } => Some((k.as_str(), *typ_value, *default)),
                _ => None,
            })
            .collect();

        let type_checkers: Vec<Option<TypeCompiled<Value<'v>>>> = custom_args
            .iter()
            .map(|(_, typ_value, _)| {
                typ_value
                    .map(|fv| TypeCompiled::new(fv.to_value(), eval.heap()))
                    .transpose()
                    .map_err(|e| starlark::Error::new_other(anyhow::anyhow!("{:?}", e)))
            })
            .collect::<starlark::Result<_>>()?;

        args.no_positional_args(eval.heap())?;
        let kwargs = args.names_map()?;

        let mut values: Vec<Cell<Value<'v>>> = Vec::with_capacity(custom_args.len());
        for ((field_name, _, default), tc) in custom_args.iter().zip(type_checkers.iter()) {
            let value = if let Some(v) = kwargs.get(*field_name) {
                *v
            } else if let Some(default_fv) = default {
                default_fv.to_value()
            } else {
                // No storable default (e.g. lambda default was dropped at definition time).
                // Fall back to None so the arg is accessible as ctx.args.name.
                eval.heap().alloc(starlark::values::none::NoneType)
            };

            if let Some(tc) = tc {
                if !tc.matches(value) {
                    return Err(starlark::Error::new_other(anyhow::anyhow!(
                        "Arg `{}` expected type `{}`, got `{}`",
                        field_name,
                        tc,
                        value.get_type()
                    )));
                }
            }

            values.push(Cell::new(value));
        }

        let instance = FeatureInstance {
            typ: _me,
            values: values.into_boxed_slice(),
            type_checkers: type_checkers.into_boxed_slice(),
            config_overrides: RefCell::new(SmallMap::new()),
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
        Ok(FrozenFeatureType {
            id: self.id,
            name: self.name,
            export_name: self.export_name,
            display_name: self.display_name,
            summary: self.summary,
            description: self.description,
            args: self.args, // Arg is Clone+simple, no freeze needed
            implementation_fn: self
                .implementation_fn
                .map(|f| f.freeze(freezer))
                .transpose()?,
        })
    }
}

// -----------------------------------------------------------------------------
// Custom arg index helper
// -----------------------------------------------------------------------------

/// Return the index of `name` within the Custom-only subset of the args map.
/// Custom args are stored as `FeatureInstance.values[i]` in their iteration order.
fn custom_arg_index(args: &SmallMap<String, Arg>, name: &str) -> Option<usize> {
    let mut idx = 0;
    for (k, v) in args.iter() {
        if matches!(v, Arg::Custom { .. }) {
            if k.as_str() == name {
                return Some(idx);
            }
            idx += 1;
        }
    }
    None
}

// -----------------------------------------------------------------------------
// FeatureInstance
// -----------------------------------------------------------------------------

/// An instance of a feature type, containing field values.
#[derive(Debug, ProvidesStaticType, NoSerialize, Allocative)]
pub struct FeatureInstance<'v> {
    /// The feature type this instance belongs to.
    pub(crate) typ: Value<'v>,
    /// Field values in the same order as the type's Custom-arg definitions.
    /// Only `Custom` args are stored here; CLI-exposed args come from the `args_builder`.
    #[allocative(skip)]
    pub(crate) values: Box<[Cell<Value<'v>>]>,
    /// Fresh type checkers created at construction time. `None` when the type annotation
    /// could not be frozen (e.g. `typing.Callable[[str], str]`) — type checking is skipped.
    #[allocative(skip)]
    pub(crate) type_checkers: Box<[Option<TypeCompiled<Value<'v>>>]>,
    /// Config.axl overrides for CLI-exposed args, set via `ctx.features[X].args.name = value`.
    /// Applied at feature invocation time for keys the user did NOT explicitly pass on the CLI.
    #[allocative(skip)]
    pub(crate) config_overrides: RefCell<SmallMap<String, OwnedFrozenValue>>,
}

impl<'v> Display for FeatureInstance<'v> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}(", self.typ)?;
        let mut first = true;
        if let Some(feat_type) = self.typ.downcast_ref::<FeatureType>() {
            let custom_names = feat_type
                .args
                .iter()
                .filter(|(_, v)| matches!(v, Arg::Custom { .. }))
                .map(|(k, _)| k.as_str());
            for (name, value) in custom_names.zip(self.values.iter()) {
                if !first {
                    write!(f, ", ")?;
                }
                write!(f, "{}={}", name, value.get())?;
                first = false;
            }
        } else if let Some(frozen_type) = self.typ.downcast_ref::<FrozenFeatureType>() {
            let custom_names = frozen_type
                .args
                .iter()
                .filter(|(_, v)| matches!(v, Arg::Custom { .. }))
                .map(|(k, _)| k.as_str());
            for (name, value) in custom_names.zip(self.values.iter()) {
                if !first {
                    write!(f, ", ")?;
                }
                write!(f, "{}={}", name, value.get())?;
                first = false;
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
            if let Some(tc) = tc {
                tc.trace(tracer);
            }
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
            // Return the stored override if present, otherwise the arg default.
            if let Some(val) = self
                .config_overrides
                .borrow()
                .get("enabled")
                .and_then(|o| o.value().unpack_frozen())
                .map(|fv| fv.to_value())
            {
                return Some(val);
            }
            let default = if let Some(ft) = self.typ.downcast_ref::<FeatureType>() {
                matches!(
                    ft.args.get("enabled"),
                    Some(Arg::Boolean { default: true, .. })
                )
            } else if let Some(ft) = self.typ.downcast_ref::<FrozenFeatureType>() {
                matches!(
                    ft.args.get("enabled"),
                    Some(Arg::Boolean { default: true, .. })
                )
            } else {
                true
            };
            return Some(heap.alloc(default));
        }
        let idx = if let Some(feat_type) = self.typ.downcast_ref::<FeatureType>() {
            custom_arg_index(&feat_type.args, attribute)
        } else if let Some(frozen_type) = self.typ.downcast_ref::<FrozenFeatureType>() {
            custom_arg_index(&frozen_type.args, attribute)
        } else {
            None
        };
        idx.map(|i| self.values[i].get())
    }

    fn set_attr(&self, attribute: &str, value: Value<'v>) -> starlark::Result<()> {
        if attribute == "enabled" {
            value.unpack_bool().ok_or_else(|| {
                starlark::Error::new_other(anyhow::anyhow!(
                    "`enabled` must be a bool, got `{}`",
                    value.get_type()
                ))
            })?;
            let frozen = freeze_value(value)
                .map_err(|e| starlark::Error::new_other(anyhow::anyhow!("{:?}", e)))?;
            self.config_overrides
                .borrow_mut()
                .insert("enabled".to_owned(), frozen);
            return Ok(());
        }
        let idx = if let Some(feat_type) = self.typ.downcast_ref::<FeatureType>() {
            custom_arg_index(&feat_type.args, attribute)
        } else if let Some(frozen_type) = self.typ.downcast_ref::<FrozenFeatureType>() {
            custom_arg_index(&frozen_type.args, attribute)
        } else {
            return Err(starlark::Error::new_other(anyhow::anyhow!(
                "Invalid feature type"
            )));
        };

        let idx = idx.ok_or_else(|| {
            starlark::Error::new_other(anyhow::anyhow!(
                "Feature {} has no attr `{}`",
                self.typ,
                attribute
            ))
        })?;

        if let Some(tc) = &self.type_checkers[idx] {
            if !tc.matches(value) {
                return Err(starlark::Error::new_other(anyhow::anyhow!(
                    "Arg `{}` expected type `{}`, got `{}`",
                    attribute,
                    tc,
                    value.get_type()
                )));
            }
        }

        self.values[idx].set(value);
        Ok(())
    }

    fn has_attr(&self, attribute: &str, _heap: Heap<'v>) -> bool {
        if attribute == "enabled" || attribute == "args" {
            return true;
        }
        if let Some(feat_type) = self.typ.downcast_ref::<FeatureType>() {
            custom_arg_index(&feat_type.args, attribute).is_some()
        } else if let Some(frozen_type) = self.typ.downcast_ref::<FrozenFeatureType>() {
            custom_arg_index(&frozen_type.args, attribute).is_some()
        } else {
            false
        }
    }

    fn dir_attr(&self) -> Vec<String> {
        let mut result = vec!["enabled".to_string(), "args".to_string()];
        if let Some(feat_type) = self.typ.downcast_ref::<FeatureType>() {
            result.extend(
                feat_type
                    .args
                    .iter()
                    .filter(|(_, v)| matches!(v, Arg::Custom { .. }))
                    .map(|(k, _)| k.clone()),
            );
        } else if let Some(frozen_type) = self.typ.downcast_ref::<FrozenFeatureType>() {
            result.extend(
                frozen_type
                    .args
                    .iter()
                    .filter(|(_, v)| matches!(v, Arg::Custom { .. }))
                    .map(|(k, _)| k.clone()),
            );
        }
        result
    }

    fn get_methods() -> Option<&'static Methods> {
        static RES: MethodsStatic = MethodsStatic::new();
        RES.methods(feature_instance_methods)
    }
}

#[starlark_module]
fn feature_instance_methods(builder: &mut MethodsBuilder) {
    /// Arg accessor for overriding CLI-exposed feature args from `config.axl`.
    ///
    /// **Writing** stores an override that is applied at feature invocation time,
    /// but only for args the user did NOT explicitly pass on the command line:
    /// ```starlark
    /// ctx.features[GithubStatusChecks].args.enabled = True
    /// ```
    ///
    /// **Reading** returns the current stored override, or the arg's default if none is set:
    /// ```starlark
    /// enabled = ctx.features[GithubStatusChecks].args.enabled  # True or False
    /// ```
    ///
    /// Precedence: explicit CLI flag > config.axl override > feature default.
    #[starlark(attribute)]
    fn args<'v>(this: Value<'v>) -> anyhow::Result<FeatureInstanceArgs<'v>> {
        this.downcast_ref::<FeatureInstance>()
            .ok_or_else(|| anyhow::anyhow!("internal: not a FeatureInstance"))?;
        Ok(FeatureInstanceArgs { instance: this })
    }
}

// -----------------------------------------------------------------------------
// FrozenFeatureInstance
// -----------------------------------------------------------------------------

#[derive(Debug, ProvidesStaticType, NoSerialize, Allocative)]
pub struct FrozenFeatureInstance {
    pub(crate) typ: FrozenValue,
    pub(crate) values: Box<[FrozenValue]>,
    #[allocative(skip)]
    pub(crate) config_overrides: SmallMap<String, OwnedFrozenValue>,
}

impl Display for FrozenFeatureInstance {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}(", self.typ)?;
        let mut first = true;
        if let Some(frozen_type) = self.typ.downcast_ref::<FrozenFeatureType>() {
            let custom_names = frozen_type
                .args
                .iter()
                .filter(|(_, v)| matches!(v, Arg::Custom { .. }))
                .map(|(k, _)| k.as_str());
            for (name, value) in custom_names.zip(self.values.iter()) {
                if !first {
                    write!(f, ", ")?;
                }
                write!(f, "{}={}", name, value)?;
                first = false;
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
            if let Some(val) = self
                .config_overrides
                .get("enabled")
                .and_then(|o| o.value().unpack_frozen())
                .map(|fv| fv.to_value())
            {
                return Some(val);
            }
            let default = if let Some(ft) = self.typ.downcast_ref::<FrozenFeatureType>() {
                matches!(
                    ft.args.get("enabled"),
                    Some(Arg::Boolean { default: true, .. })
                )
            } else {
                true
            };
            return Some(heap.alloc(default));
        }
        if let Some(frozen_type) = self.typ.downcast_ref::<FrozenFeatureType>() {
            if let Some(idx) = custom_arg_index(&frozen_type.args, attribute) {
                return Some(self.values[idx].to_value());
            }
        }
        None
    }

    fn has_attr(&self, attribute: &str, _heap: Heap<'v>) -> bool {
        if attribute == "enabled" || attribute == "args" {
            return true;
        }
        if let Some(frozen_type) = self.typ.downcast_ref::<FrozenFeatureType>() {
            custom_arg_index(&frozen_type.args, attribute).is_some()
        } else {
            false
        }
    }

    fn dir_attr(&self) -> Vec<String> {
        let mut result = vec!["enabled".to_string(), "args".to_string()];
        if let Some(frozen_type) = self.typ.downcast_ref::<FrozenFeatureType>() {
            result.extend(
                frozen_type
                    .args
                    .iter()
                    .filter(|(_, v)| matches!(v, Arg::Custom { .. }))
                    .map(|(k, _)| k.clone()),
            );
        }
        result
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
            config_overrides: self.config_overrides.into_inner(),
        })
    }
}

// -----------------------------------------------------------------------------
// FeatureInstanceArgs — accessor returned by `ctx.features[X].args`
// -----------------------------------------------------------------------------

/// Accessor returned by `.args` on a `FeatureInstance`.
///
/// Holds a back-reference to the instance so that writes (`f.args.enabled = True`)
/// are stored in `FeatureInstance::config_overrides` and applied at invocation time
/// for any arg the user did NOT explicitly provide on the command line.
#[derive(Debug, ProvidesStaticType, NoSerialize, Allocative)]
pub struct FeatureInstanceArgs<'v> {
    #[allocative(skip)]
    instance: Value<'v>,
}

impl<'v> Display for FeatureInstanceArgs<'v> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "<FeatureInstanceArgs>")
    }
}

unsafe impl<'v> Trace<'v> for FeatureInstanceArgs<'v> {
    fn trace(&mut self, tracer: &Tracer<'v>) {
        self.instance.trace(tracer);
    }
}

impl<'v> AllocValue<'v> for FeatureInstanceArgs<'v> {
    fn alloc_value(self, heap: Heap<'v>) -> Value<'v> {
        heap.alloc_complex(self)
    }
}

impl<'v> Freeze for FeatureInstanceArgs<'v> {
    type Frozen = FrozenFeatureInstanceArgs;

    fn freeze(self, freezer: &Freezer) -> Result<Self::Frozen, FreezeError> {
        Ok(FrozenFeatureInstanceArgs {
            instance: self.instance.freeze(freezer)?,
        })
    }
}

/// Frozen version of `FeatureInstanceArgs` — read-only after freezing.
#[derive(Debug, ProvidesStaticType, NoSerialize, Allocative)]
pub struct FrozenFeatureInstanceArgs {
    #[allocative(skip)]
    instance: FrozenValue,
}

impl Display for FrozenFeatureInstanceArgs {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "<FeatureInstanceArgs>")
    }
}

starlark::starlark_simple_value!(FrozenFeatureInstanceArgs);

#[starlark_value(type = "FeatureInstanceArgs")]
impl<'v> StarlarkValue<'v> for FrozenFeatureInstanceArgs {
    type Canonical = FeatureInstanceArgs<'v>;

    fn get_attr(&self, attribute: &str, heap: Heap<'v>) -> Option<Value<'v>> {
        let inst = self.instance.downcast_ref::<FrozenFeatureInstance>()?;
        // Return stored override if present.
        if let Some(val) = inst
            .config_overrides
            .get(attribute)
            .and_then(|owned| owned.value().unpack_frozen())
            .map(|fv| fv.to_value())
        {
            return Some(val);
        }
        // Fall back to the arg's default.
        if let Some(ft) = inst.typ.downcast_ref::<FrozenFeatureType>() {
            let arg = ft
                .args
                .get(attribute)
                .filter(|a| !matches!(a, Arg::Custom { .. }))?;
            return Some(feature_arg_default_value(arg, heap));
        }
        None
    }

    fn dir_attr(&self) -> Vec<String> {
        if let Some(inst) = self.instance.downcast_ref::<FrozenFeatureInstance>() {
            if let Some(ft) = inst.typ.downcast_ref::<FrozenFeatureType>() {
                return ft
                    .args
                    .iter()
                    .filter(|(_, v)| !matches!(v, Arg::Custom { .. }))
                    .map(|(k, _)| k.clone())
                    .collect();
            }
        }
        vec![]
    }
}

/// Return the default value of a CLI-exposed `Arg` allocated on `heap`.
fn feature_arg_default_value<'v>(arg: &Arg, heap: Heap<'v>) -> Value<'v> {
    match arg {
        Arg::Custom { default, .. } => default
            .map(|fv| fv.to_value())
            .unwrap_or_else(|| heap.alloc(starlark::values::none::NoneType)),
        Arg::String { default, .. } => heap.alloc_str(default).to_value(),
        Arg::Boolean { default, .. } => heap.alloc(*default),
        Arg::Int { default, .. } => heap.alloc(*default),
        Arg::UInt { default, .. } => heap.alloc(*default),
        Arg::Positional { default, .. } => {
            let items: Vec<&str> = default
                .as_deref()
                .unwrap_or(&[])
                .iter()
                .map(|s| s.as_str())
                .collect();
            heap.alloc(AllocList(items))
        }
        Arg::TrailingVarArgs { .. } => heap.alloc(AllocList(Vec::<&str>::new())),
        Arg::StringList { default, .. } => heap.alloc(AllocList(
            default.iter().map(|s| s.as_str()).collect::<Vec<_>>(),
        )),
        Arg::BooleanList { default, .. } => heap.alloc(AllocList(default.iter().copied())),
        Arg::IntList { default, .. } => heap.alloc(AllocList(default.iter().copied())),
        Arg::UIntList { default, .. } => heap.alloc(AllocList(default.iter().copied())),
    }
}

/// Look up a CLI-exposed arg by name from a FeatureType or FrozenFeatureType value.
/// Returns `None` for Custom args and for unknown arg names.
fn feature_cli_arg<'a>(typ: &Value, name: &str) -> Option<Arg> {
    if let Some(ft) = typ.downcast_ref::<FeatureType>() {
        ft.args
            .get(name)
            .filter(|a| !matches!(a, Arg::Custom { .. }))
            .cloned()
    } else if let Some(ft) = typ.downcast_ref::<FrozenFeatureType>() {
        ft.args
            .get(name)
            .filter(|a| !matches!(a, Arg::Custom { .. }))
            .cloned()
    } else {
        None
    }
}

#[starlark_value(type = "FeatureInstanceArgs")]
impl<'v> StarlarkValue<'v> for FeatureInstanceArgs<'v> {
    fn set_attr(&self, attribute: &str, value: Value<'v>) -> starlark::Result<()> {
        let inst = self
            .instance
            .downcast_ref::<FeatureInstance>()
            .ok_or_else(|| {
                starlark::Error::new_other(anyhow::anyhow!("internal: not a FeatureInstance"))
            })?;

        let Some(arg) = feature_cli_arg(&inst.typ, attribute) else {
            return ValueError::unsupported(self, &format!(".{}=", attribute));
        };

        // Type-check the incoming value.
        let expected = match &arg {
            Arg::Custom { .. } => unreachable!("filtered above"),
            Arg::String { .. } | Arg::Positional { .. } => "string",
            Arg::Boolean { .. } => "bool",
            Arg::Int { .. } | Arg::UInt { .. } => "int",
            Arg::StringList { .. } | Arg::TrailingVarArgs { .. } => "list",
            Arg::BooleanList { .. } | Arg::IntList { .. } | Arg::UIntList { .. } => "list",
        };
        let actual = value.get_type();
        if actual != expected {
            return Err(starlark::Error::new_other(anyhow::anyhow!(
                "feature arg {:?}: expected type `{}`, got `{}`",
                attribute,
                expected,
                actual
            )));
        }
        // Validate allowed values for string args.
        if let Arg::String {
            values: Some(valid),
            ..
        } = &arg
        {
            if let Some(s) = value.unpack_str() {
                if !valid.iter().any(|v| v == s) {
                    return Err(starlark::Error::new_other(anyhow::anyhow!(
                        "feature arg {:?}: invalid value `{}`, expected one of: {}",
                        attribute,
                        s,
                        valid
                            .iter()
                            .map(|v| format!("`{}`", v))
                            .collect::<Vec<_>>()
                            .join(", ")
                    )));
                }
            }
        }

        let frozen = freeze_value(value)
            .map_err(|e| starlark::Error::new_other(anyhow::anyhow!("{:?}", e)))?;
        inst.config_overrides
            .borrow_mut()
            .insert(attribute.to_owned(), frozen);
        Ok(())
    }

    fn get_attr(&self, attribute: &str, heap: Heap<'v>) -> Option<Value<'v>> {
        let inst = self.instance.downcast_ref::<FeatureInstance>()?;

        // Return stored override if present.
        if let Some(val) = inst
            .config_overrides
            .borrow()
            .get(attribute)
            .and_then(|owned| owned.value().unpack_frozen())
            .map(|fv| fv.to_value())
        {
            return Some(val);
        }

        // Fall back to the arg's default.
        let arg = feature_cli_arg(&inst.typ, attribute)?;
        Some(feature_arg_default_value(&arg, heap))
    }

    fn has_attr(&self, attribute: &str, _heap: Heap<'v>) -> bool {
        if let Some(inst) = self.instance.downcast_ref::<FeatureInstance>() {
            feature_cli_arg(&inst.typ, attribute).is_some()
        } else {
            false
        }
    }

    fn dir_attr(&self) -> Vec<String> {
        if let Some(inst) = self.instance.downcast_ref::<FeatureInstance>() {
            if let Some(ft) = inst.typ.downcast_ref::<FeatureType>() {
                return ft
                    .args
                    .iter()
                    .filter(|(_, v)| !matches!(v, Arg::Custom { .. }))
                    .map(|(k, _)| k.clone())
                    .collect();
            }
            if let Some(ft) = inst.typ.downcast_ref::<FrozenFeatureType>() {
                return ft
                    .args
                    .iter()
                    .filter(|(_, v)| !matches!(v, Arg::Custom { .. }))
                    .map(|(k, _)| k.clone())
                    .collect();
            }
        }
        vec![]
    }
}

// -----------------------------------------------------------------------------
// Helpers
// -----------------------------------------------------------------------------

/// Extract the display name from a FeatureType or FrozenFeatureType value.
/// Falls back to splitting the CamelCase export name into Title Case words if no explicit
/// `display_name` was given (e.g. `"ArtifactUpload"` → `"Artifact Upload"`).
pub fn extract_feature_display_name(value: Value<'_>) -> Option<String> {
    if let Some(ft) = value.downcast_ref::<FeatureType>() {
        if !ft.display_name.is_empty() {
            return Some(ft.display_name.clone());
        }
        return ft.export_name.as_deref().map(camel_to_display_name);
    }
    if let Some(ft) = value.downcast_ref::<FrozenFeatureType>() {
        if !ft.display_name.is_empty() {
            return Some(ft.display_name.clone());
        }
        return ft.export_name.as_deref().map(camel_to_display_name);
    }
    None
}

/// Extract the CamelCase export variable name (e.g. `"ArtifactUpload"`).
/// Used in help text context lines like "feature defined in ...".
pub fn extract_feature_identifier(value: Value<'_>) -> Option<String> {
    if let Some(ft) = value.downcast_ref::<FeatureType>() {
        return ft.export_name.clone();
    }
    if let Some(ft) = value.downcast_ref::<FrozenFeatureType>() {
        return ft.export_name.clone();
    }
    None
}

/// Extract the kebab-case name used as the CLI arg prefix (e.g. `"artifact-upload"`).
/// Returns `None` for anonymous features where `export_as` was never called and no
/// explicit `name` was given.
pub fn extract_feature_name(value: Value<'_>) -> Option<String> {
    if let Some(ft) = value.downcast_ref::<FeatureType>() {
        if !ft.name.is_empty() {
            return Some(ft.name.clone());
        }
        return None;
    }
    if let Some(ft) = value.downcast_ref::<FrozenFeatureType>() {
        if !ft.name.is_empty() {
            return Some(ft.name.clone());
        }
        return None;
    }
    None
}

/// Extract the one-line summary from a FeatureType or FrozenFeatureType value.
pub fn extract_feature_summary(value: Value<'_>) -> Option<String> {
    if let Some(ft) = value.downcast_ref::<FeatureType>() {
        return Some(ft.summary.clone());
    }
    if let Some(ft) = value.downcast_ref::<FrozenFeatureType>() {
        return Some(ft.summary.clone());
    }
    None
}

/// Extract the extended description from a FeatureType or FrozenFeatureType value.
pub fn extract_feature_description(value: Value<'_>) -> Option<String> {
    if let Some(ft) = value.downcast_ref::<FeatureType>() {
        return Some(ft.description.clone());
    }
    if let Some(ft) = value.downcast_ref::<FrozenFeatureType>() {
        return Some(ft.description.clone());
    }
    None
}

/// Extract the CLI args from a FeatureType or FrozenFeatureType value.
/// Filters the args map to return only CLI-exposed entries (excludes Custom args).
/// Returns None if the value is not a feature type, Some (possibly empty map) otherwise.
pub fn extract_feature_args(value: Value<'_>) -> Option<SmallMap<String, Arg>> {
    if let Some(ft) = value.downcast_ref::<FeatureType>() {
        Some(
            ft.args
                .iter()
                .filter(|(_, v)| v.is_cli_exposed())
                .map(|(k, v)| (k.clone(), v.clone()))
                .collect(),
        )
    } else if let Some(ft) = value.downcast_ref::<FrozenFeatureType>() {
        Some(
            ft.args
                .iter()
                .filter(|(_, v)| v.is_cli_exposed())
                .map(|(k, v)| (k.clone(), v.clone()))
                .collect(),
        )
    } else {
        None
    }
}

/// Apply config.axl overrides for CLI-exposed feature args.
///
/// `cli_args` is pre-populated with all CLI-parsed values (including Clap defaults).
/// `explicit_args` contains only the keys the user explicitly passed on the command line.
///
/// For each override stored in the feature instance's `config_overrides`, this function
/// inserts it into `cli_args` — but only if the key is NOT in `explicit_args`.
/// This preserves the precedence: explicit CLI flag > config.axl override > Clap default.
pub fn apply_feature_config_overrides<'v>(
    instance_value: Value<'v>,
    mut cli_args: CliArgs<'v>,
    explicit_args: &CliArgs<'v>,
) -> CliArgs<'v> {
    if let Some(inst) = instance_value.downcast_ref::<FeatureInstance>() {
        for (k, owned) in inst.config_overrides.borrow().iter() {
            if !explicit_args.contains_key(k.as_str()) {
                if let Some(fv) = owned.value().unpack_frozen() {
                    cli_args.insert(k.clone(), fv.to_value());
                }
            }
        }
    }
    cli_args
}

/// Serialize a feature instance's `config_overrides` into Clap-compatible effective defaults.
///
/// The returned map has the same shape as the task equivalent: arg_name -> Vec<String>.
/// Booleans are lowercased ("true"/"false") because Clap's `value_parser!(bool)` expects
/// lowercase while Starlark's `to_string()` produces "True"/"False". Strings are
/// unpacked via `unpack_str()` to get the raw value; `to_string()` on a Starlark
/// string returns the repr form (quoted), which Clap rejects when a `values=[...]`
/// constraint is in play.
/// Convert a Starlark value to a Clap-parsable argument string.
///
/// - Strings are unpacked via `unpack_str()` — `Value::to_string()` on a
///   Starlark string returns the repr form (e.g. `"failed"` with embedded
///   quotes), which Clap rejects when a `values=[...]` constraint is in play.
/// - Bools get lowercased because Clap's `value_parser!(bool)` expects
///   lowercase while Starlark's `to_string()` produces `True`/`False`.
/// - Ints, lists, and other types pass through as `Value::to_string()`.
pub fn stringify_arg_value(v: Value<'_>) -> String {
    if let Some(s) = v.unpack_str() {
        return s.to_owned();
    }
    let s = v.to_string();
    if v.get_type() == "bool" {
        s.to_lowercase()
    } else {
        s
    }
}

pub fn feature_instance_effective_defaults(
    instance_value: Value<'_>,
) -> std::collections::HashMap<String, Vec<String>> {
    let mut out = std::collections::HashMap::new();
    let borrow;
    let iter: Box<dyn Iterator<Item = (&String, &OwnedFrozenValue)>> =
        if let Some(inst) = instance_value.downcast_ref::<FeatureInstance>() {
            borrow = inst.config_overrides.borrow();
            Box::new(borrow.iter())
        } else if let Some(inst) = instance_value.downcast_ref::<FrozenFeatureInstance>() {
            Box::new(inst.config_overrides.iter())
        } else {
            return out;
        };
    for (k, owned) in iter {
        let v = owned.value();
        use starlark::values::list::ListRef;
        let elems: Vec<String> = if let Some(list) = ListRef::from_value(v) {
            list.iter().map(stringify_arg_value).collect()
        } else {
            vec![stringify_arg_value(v)]
        };
        out.insert(k.clone(), elems);
    }
    out
}

/// Populate `Custom` arg values from a feature instance into the CLI-sourced `Args` map.
///
/// `cli_args` is pre-populated with CLI-parsed values for CLI-exposed args. This function
/// inserts the values of `Custom` args (set in `config.axl`) so that the feature
/// implementation can read all args uniformly through a single `ctx.args` map.
pub fn populate_feature_custom_args<'v>(
    type_value: Value<'v>,
    instance_value: Value<'v>,
    mut cli_args: CliArgs<'v>,
) -> CliArgs<'v> {
    if let Some(ft) = type_value.downcast_ref::<FeatureType>() {
        if let Some(inst) = instance_value.downcast_ref::<FeatureInstance>() {
            let custom_names: Vec<&str> = ft
                .args
                .iter()
                .filter(|(_, v)| matches!(v, Arg::Custom { .. }))
                .map(|(k, _)| k.as_str())
                .collect();
            for (name, cell) in custom_names.into_iter().zip(inst.values.iter()) {
                cli_args.insert(name.to_owned(), cell.get());
            }
        }
    } else if let Some(ft) = type_value.downcast_ref::<FrozenFeatureType>() {
        if let Some(inst) = instance_value.downcast_ref::<FrozenFeatureInstance>() {
            let custom_names: Vec<&str> = ft
                .args
                .iter()
                .filter(|(_, v)| matches!(v, Arg::Custom { .. }))
                .map(|(k, _)| k.as_str())
                .collect();
            for (name, fv) in custom_names.into_iter().zip(inst.values.iter()) {
                cli_args.insert(name.to_owned(), fv.to_value());
            }
        }
    }
    cli_args
}

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
    /// Feature CLI args are injected into every task subcommand on the CLI. Only named
    /// optional flags are supported — positional args and `required = true` are not
    /// allowed because features apply globally and would break any task that doesn't
    /// supply the flag.
    ///
    /// Every feature automatically gets an `enabled` CLI arg. It shows up as
    /// `--{name}:enabled` on the command line and is accessible as `ctx.args.enabled`
    /// in the implementation. Set `enabled = False` for opt-in features.
    /// Override in config.axl via `ctx.features[MyFeature].enabled = True`.
    ///
    /// ## Naming
    ///
    /// Features must be exported as **CamelCase** (`ArtifactUpload`).
    /// This is enforced at definition time. Features are referenced as type keys
    /// (`ctx.features[ArtifactUpload]`), mirroring Bazel's provider convention
    /// (`dep[CcInfo]`); CamelCase signals this type-key role.
    ///
    /// The `name` field sets the kebab-case slug used as a prefix for every CLI arg this
    /// feature declares: a feature named `"artifact-upload"` with arg `mode` exposes
    /// `--artifact-upload-mode`. The name is auto-derived from the CamelCase export name
    /// via `to_command_name` (`ArtifactUpload` → `artifact-upload`) if not set explicitly.
    /// Override with `name = "s3"` when the auto-derived form is undesirable (e.g. for
    /// acronym-heavy names like `GitHubActions` → `git-hub-actions`).
    ///
    /// `display_name` overrides the Title Case name shown in CLI help section headings.
    ///
    /// ## Arg names
    ///
    /// Arg keys must be `snake_case` (`[a-z][a-z0-9_]*`). There are two kinds:
    ///
    /// - **CLI args** (`args.string(...)`, `args.boolean(...)`, etc.) — exposed as
    ///   `--{name}-{arg}` flags on every task subcommand; must be optional (no `required = true`).
    /// - **Config-only args** (`args.custom(type, default = …)`) — set in `config.axl` only, not shown in help.
    ///
    /// Both kinds are accessible as `ctx.args.arg_name` in the implementation.
    ///
    /// ## Example
    ///
    /// ```starlark
    /// def _impl(ctx: FeatureContext):
    ///     ctx.fragments[BazelFragment].build_end.append(
    ///         lambda task_ctx, state: upload_artifacts(
    ///             task_ctx, ctx.args.bucket, ctx.args.mode
    ///         )
    ///     )
    ///
    /// ArtifactUpload = feature(
    ///     # name auto-derived as "artifact-upload"; override with name = "s3" if preferred
    ///     display_name = "Artifact Upload",
    ///     summary = "Upload build artifacts to S3 storage",
    ///     implementation = _impl,
    ///     args = {
    ///         "bucket": args.custom(str | None, default = None),  # config.axl only
    ///         "mode":   args.string(default = "auto"),             # CLI flag: --artifact-upload-mode
    ///     },
    /// )
    /// ```
    fn feature<'v>(
        #[starlark(require = named)] implementation: Value<'v>,
        #[starlark(require = named, default = String::new())] name: String,
        #[starlark(require = named, default = String::new())] display_name: String,
        #[starlark(require = named, default = String::new())] summary: String,
        #[starlark(require = named, default = String::new())] description: String,
        #[starlark(require = named, default = true)] enabled: bool,
        #[starlark(require = named, default = UnpackDictEntries::default())]
        args: UnpackDictEntries<String, Value<'v>>,
        _eval: &mut starlark::eval::Evaluator<'v, '_, '_>,
    ) -> anyhow::Result<FeatureType<'v>> {
        // Validate explicit name; derivation from the export name happens in export_as.
        if !name.is_empty() {
            validate_command_name(&name, "feature name").map_err(|e| anyhow::anyhow!(e))?;
        }

        // The implicit `enabled` arg is always first in the map.
        let mut args_ = SmallMap::with_capacity(args.entries.len() + 1);
        args_.insert(
            "enabled".to_owned(),
            Arg::Boolean {
                required: false,
                default: enabled,
                short: None,
                long: None,
                description: Some(if enabled {
                    "Set to false to disable this feature".to_owned()
                } else {
                    "Set to true to enable this feature".to_owned()
                }),
            },
        );

        for (arg_name, value) in args.entries.into_iter() {
            if arg_name == "enabled" {
                return Err(anyhow::anyhow!(
                    "feature arg \"enabled\" is implicit — remove it from `args` and use \
                     `enabled = True/False` on the `feature()` call instead"
                ));
            }
            validate_arg_name(&arg_name).map_err(|e| anyhow::anyhow!("feature {}", e))?;
            let cli_arg = value.downcast_ref::<Arg>().ok_or_else(|| {
                anyhow::anyhow!(
                    "feature arg {:?}: expected args.string/boolean/... or args.custom(...), got '{}'. \
                     All feature args must have a default — use args.custom(type, default = …) for config-only args \
                     or args.string(default = …) etc. for CLI flags",
                    arg_name,
                    value.get_type()
                )
            })?;
            if matches!(
                cli_arg,
                Arg::Positional { .. } | Arg::TrailingVarArgs { .. }
            ) {
                return Err(anyhow::anyhow!(
                    "feature arg {:?}: positional args are not allowed in features",
                    arg_name
                ));
            }
            if cli_arg.is_required() {
                return Err(anyhow::anyhow!(
                    "feature arg {:?}: CLI args in features must be optional (required = true is not allowed); \
                     features inject args into every task subcommand so required flags would break all tasks",
                    arg_name
                ));
            }
            args_.insert(arg_name, cli_arg.clone());
        }

        Ok(FeatureType {
            id: next_feature_type_id(),
            name,
            export_name: None,
            display_name,
            summary,
            description,
            args: args_,
            implementation_fn: Some(implementation),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // ── stringify_arg_value ──────────────────────────────────────────────────
    //
    // Critical correctness: Clap's `values=[...]` constraint compares the parsed
    // input byte-for-byte against the allowed list. A Starlark string passed
    // through `Value::to_string()` includes the surrounding quotes, which makes
    // Clap reject even valid choices. Regression test for that bug.

    #[test]
    fn stringify_string_unquoted() {
        use starlark::environment::Module;
        Module::with_temp_heap(|module| {
            let heap = module.heap();
            let v = heap.alloc("failed");
            assert_eq!(stringify_arg_value(v), "failed");
        });
    }

    #[test]
    fn stringify_bool_lowercased() {
        use starlark::environment::Module;
        Module::with_temp_heap(|module| {
            let heap = module.heap();
            let t = starlark::values::Value::new_bool(true);
            let f = starlark::values::Value::new_bool(false);
            let _ = heap;
            assert_eq!(stringify_arg_value(t), "true");
            assert_eq!(stringify_arg_value(f), "false");
        });
    }

    #[test]
    fn stringify_int_plain() {
        use starlark::environment::Module;
        Module::with_temp_heap(|module| {
            let heap = module.heap();
            let v = heap.alloc(42i32);
            assert_eq!(stringify_arg_value(v), "42");
        });
    }

    // ── to_command_name ──────────────────────────────────────────────────────

    #[test]
    fn command_name_snake_case() {
        assert_eq!(to_command_name("axl_add"), "axl-add");
        assert_eq!(to_command_name("remote_cache"), "remote-cache");
        assert_eq!(
            to_command_name("bazel_startup_flags"),
            "bazel-startup-flags"
        );
    }

    #[test]
    fn command_name_single_word() {
        assert_eq!(to_command_name("build"), "build");
        assert_eq!(to_command_name("Build"), "build");
    }

    #[test]
    fn command_name_camel_case() {
        assert_eq!(to_command_name("AxlAdd"), "axl-add");
        assert_eq!(to_command_name("DeliveryTask"), "delivery-task");
        assert_eq!(to_command_name("UserTaskManual"), "user-task-manual");
    }

    #[test]
    fn command_name_acronym_prefix() {
        assert_eq!(to_command_name("CIBuild"), "ci-build");
        assert_eq!(to_command_name("ACKey"), "ac-key");
    }

    #[test]
    fn command_name_acronym_run() {
        assert_eq!(to_command_name("HTTPSRedirect"), "https-redirect");
        assert_eq!(to_command_name("XMLParser"), "xml-parser");
    }

    #[test]
    fn command_name_digit_boundary() {
        assert_eq!(to_command_name("S3Upload"), "s3-upload");
        assert_eq!(to_command_name("x86_64"), "x86-64");
        assert_eq!(to_command_name("task1"), "task1");
    }

    #[test]
    fn command_name_leading_underscore_stripped() {
        // _-prefixed names are private in Starlark and won't be exported,
        // but to_command_name strips them gracefully rather than panicking.
        assert_eq!(to_command_name("_private"), "private");
    }

    // ── to_display_name ──────────────────────────────────────────────────────

    #[test]
    fn display_name_from_kebab() {
        assert_eq!(to_display_name("axl-add"), "Axl Add");
        assert_eq!(to_display_name("ci-build"), "Ci Build");
        assert_eq!(to_display_name("s3-upload"), "S3 Upload");
    }

    #[test]
    fn display_name_from_snake() {
        assert_eq!(to_display_name("artifact_upload"), "Artifact Upload");
        assert_eq!(to_display_name("bazel_defaults"), "Bazel Defaults");
    }

    #[test]
    fn display_name_single_word() {
        assert_eq!(to_display_name("build"), "Build");
    }

    // ── camel_to_display_name ────────────────────────────────────────────────

    #[test]
    fn camel_display_name_basic() {
        assert_eq!(camel_to_display_name("ArtifactUpload"), "Artifact Upload");
        assert_eq!(camel_to_display_name("BazelDefaults"), "Bazel Defaults");
    }

    #[test]
    fn camel_display_name_acronym() {
        assert_eq!(camel_to_display_name("CIBuild"), "Ci Build");
        assert_eq!(camel_to_display_name("S3Upload"), "S3 Upload");
    }

    #[test]
    fn camel_display_name_single_word() {
        assert_eq!(camel_to_display_name("Build"), "Build");
        assert_eq!(camel_to_display_name("MyConfig"), "My Config");
    }

    // ── validate_arg_name ────────────────────────────────────────────────────

    #[test]
    fn arg_name_valid() {
        for name in &[
            "a",
            "foo",
            "foo_bar",
            "foo123",
            "bazel_flags",
            "remote_cache",
        ] {
            assert!(
                validate_arg_name(name).is_ok(),
                "expected {:?} to be valid",
                name
            );
        }
    }

    #[test]
    fn arg_name_invalid_start() {
        assert!(validate_arg_name("").is_err());
        assert!(validate_arg_name("1foo").is_err());
        assert!(validate_arg_name("_foo").is_err());
        assert!(validate_arg_name("Foo").is_err());
        assert!(validate_arg_name("FOO").is_err());
    }

    #[test]
    fn arg_name_invalid_chars() {
        assert!(validate_arg_name("foo-bar").is_err()); // dashes not allowed (use snake_case)
        assert!(validate_arg_name("fooBar").is_err()); // uppercase mid-name
        assert!(validate_arg_name("foo.bar").is_err());
        assert!(validate_arg_name("foo bar").is_err());
    }

    // ── validate_command_name ────────────────────────────────────────────────

    #[test]
    fn command_name_valid() {
        for name in &["build", "axl-add", "ci-build", "s3-upload", "a", "z9"] {
            assert!(
                validate_command_name(name, "task").is_ok(),
                "expected {:?} to be valid",
                name
            );
        }
    }

    #[test]
    fn command_name_invalid_start() {
        assert!(validate_command_name("", "task").is_err());
        assert!(validate_command_name("1foo", "task").is_err());
        assert!(validate_command_name("-foo", "task").is_err());
        assert!(validate_command_name("Foo", "task").is_err());
        assert!(validate_command_name("_foo", "task").is_err());
    }

    #[test]
    fn command_name_invalid_chars() {
        assert!(validate_command_name("foo_bar", "task").is_err()); // underscores not allowed
        assert!(validate_command_name("fooBar", "task").is_err()); // uppercase
        assert!(validate_command_name("foo bar", "task").is_err());
    }

    #[test]
    fn command_name_no_trailing_or_consecutive_dashes() {
        assert!(validate_command_name("axl-add", "task").is_ok());
        assert!(validate_command_name("a-b-c", "task").is_ok());
        assert!(validate_command_name("axl-", "task").is_err()); // trailing dash
        assert!(validate_command_name("axl--add", "task").is_err()); // consecutive dashes
    }

    #[test]
    fn command_name_error_includes_kind() {
        let err = validate_command_name("foo_bar", "group").unwrap_err();
        assert!(
            err.contains("group"),
            "error should mention 'group': {}",
            err
        );
        let err = validate_command_name("foo_bar", "task").unwrap_err();
        assert!(err.contains("task"), "error should mention 'task': {}", err);
    }

    // ── validate_type_name ───────────────────────────────────────────────────

    #[test]
    fn type_name_valid() {
        for name in &[
            "A",
            "Foo",
            "FooBar",
            "ArtifactUpload",
            "MyConfig",
            "CcInfo",
            "S3Upload",
            "CIBuild",
        ] {
            assert!(
                validate_type_name(name, "feature").is_ok(),
                "expected {:?} to be valid",
                name
            );
        }
    }

    #[test]
    fn type_name_invalid_start() {
        assert!(validate_type_name("", "feature").is_err());
        assert!(validate_type_name("foo", "feature").is_err()); // lowercase start
        assert!(validate_type_name("1Foo", "feature").is_err()); // digit start
        assert!(validate_type_name("_Foo", "feature").is_err()); // underscore start
    }

    #[test]
    fn type_name_invalid_chars() {
        assert!(validate_type_name("Foo_Bar", "feature").is_err()); // underscore
        assert!(validate_type_name("Foo-Bar", "feature").is_err()); // dash
        assert!(validate_type_name("Foo Bar", "feature").is_err()); // space
        assert!(validate_type_name("Foo.Bar", "feature").is_err()); // dot
    }

    #[test]
    fn type_name_error_includes_kind() {
        let err = validate_type_name("bad_name", "feature").unwrap_err();
        assert!(
            err.contains("feature"),
            "error should mention 'feature': {}",
            err
        );
        let err = validate_type_name("bad_name", "trait").unwrap_err();
        assert!(
            err.contains("trait"),
            "error should mention 'trait': {}",
            err
        );
    }
}
