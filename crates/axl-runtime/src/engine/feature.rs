use std::cell::RefCell;
use std::path::PathBuf;

use allocative::Allocative;
use derive_more::Display;
use starlark::collections::SmallMap;
use starlark::environment::GlobalsBuilder;
use starlark::starlark_module;
use starlark::starlark_simple_value;
use starlark::typing::ParamIsRequired;
use starlark::typing::ParamSpec;
use starlark::values;
use starlark::values::FrozenValue;
use starlark::values::Heap;
use starlark::values::NoSerialize;
use starlark::values::ProvidesStaticType;
use starlark::values::StarlarkValue;
use starlark::values::Trace;
use starlark::values::Value;
use starlark::values::ValueLike;
use starlark::values::dict::UnpackDictEntries;
use starlark::values::none::NoneType;
use starlark::values::starlark_value;
use starlark::values::typing::StarlarkCallableParamSpec;

use super::arg::Arg;
use super::arguments::{Arguments, FrozenArguments};
use super::feature_context::FeatureContext;
use super::names::{
    camel_to_display_name, to_command_name, validate_arg_name, validate_command_name,
    validate_type_name,
};
use super::store::Env;

pub trait FeatureLike<'v> {
    /// Kebab slug used as a CLI arg prefix (e.g. `"artifact-upload"`). Empty if anonymous.
    fn name(&self) -> String;
    /// CamelCase Starlark variable name (e.g. `"ArtifactUpload"`), set by `export_as`.
    fn export_name(&self) -> Option<String>;
    /// One-line summary shown in CLI help.
    fn summary(&self) -> &String;
    /// Extended description shown in `--help` after the summary.
    fn description(&self) -> &String;
    /// Title Case display name for help section headings.
    fn display_name(&self) -> String;
    /// Full arg map (CLI-exposed + Custom).
    fn args(&self) -> &SmallMap<String, Arg>;
    /// Absolute path to the .axl file the feature was defined in.
    fn path(&self) -> &PathBuf;
    /// `Arguments` value carrying config.axl overrides for this feature.
    /// Live features return their mutable store directly; frozen features
    /// return the frozen variant lifted to the live heap via `to_value()`.
    fn overrides(&self) -> Value<'v>;
    /// Implementation function lifted to the live heap via `to_value()`.
    fn implementation(&self) -> Value<'v>;

    /// Returns only the CLI-exposed args (non-Custom entries), as (name, &Arg) pairs.
    fn cli_args(&self) -> Vec<(&str, &Arg)> {
        self.args()
            .iter()
            .filter(|(_, v)| v.is_cli_exposed())
            .map(|(k, v)| (k.as_str(), v))
            .collect()
    }
}

/// Cast helper: borrow anything carrying a feature as `&dyn FeatureLike<'v>`.
///
/// Implemented for `Feature` / `FrozenFeature` (trivial upcast) and for
/// `Value<'v>` (downcasts to whichever variant the value holds). Lets callers
/// iterate `FeatureMap` entries without caring whether they are live or frozen.
pub trait AsFeatureLike<'v> {
    fn as_feature(&self) -> &dyn FeatureLike<'v>;
}

impl<'v> AsFeatureLike<'v> for Feature<'v> {
    fn as_feature(&self) -> &dyn FeatureLike<'v> {
        self
    }
}

impl<'v> AsFeatureLike<'v> for FrozenFeature {
    fn as_feature(&self) -> &dyn FeatureLike<'v> {
        self
    }
}

impl<'v> AsFeatureLike<'v> for Value<'v> {
    fn as_feature(&self) -> &dyn FeatureLike<'v> {
        if let Some(f) = self.downcast_ref::<Feature<'v>>() {
            return f;
        }
        if let Some(f) = self.downcast_ref::<FrozenFeature>() {
            return f;
        }
        panic!("expected a feature value, got '{}'", self.get_type());
    }
}

#[derive(Debug, Clone, ProvidesStaticType, Display, Trace, NoSerialize, Allocative)]
#[display("<Feature>")]
pub struct Feature<'v> {
    r#impl: values::Value<'v>,
    #[allocative(skip)]
    pub(super) args: SmallMap<String, Arg>,
    pub(super) summary: String,
    pub(super) description: String,
    pub(super) display_name: RefCell<String>,
    pub(super) name: RefCell<String>,
    pub(super) export_name: RefCell<Option<String>>,
    pub(super) path: PathBuf,
    /// Mutable override store for `ctx.features[X].args.foo = ...`.
    /// Always points to an `Arguments` value on the same heap.
    pub(super) overrides: Value<'v>,
    /// `Some(fv)` for a live Feature created by `from_frozen` — used by
    /// `FeatureMap.at(X)` to match the user's `load`-imported handle to the
    /// thawed live entry. `None` for the freshly-built Feature returned by the
    /// `feature(...)` global before its module freezes.
    #[allocative(skip)]
    pub(super) frozen_handle: Option<FrozenValue>,
}

impl<'v> Feature<'v> {
    pub fn implementation(&self) -> values::Value<'v> {
        self.r#impl
    }
    pub fn args(&self) -> &SmallMap<String, Arg> {
        &self.args
    }
    pub fn summary(&self) -> &String {
        &self.summary
    }
    pub fn description(&self) -> &String {
        &self.description
    }
    pub fn display_name(&self) -> String {
        self.display_name.borrow().clone()
    }
    pub fn name(&self) -> String {
        self.name.borrow().clone()
    }
    pub fn export_name(&self) -> Option<String> {
        self.export_name.borrow().clone()
    }
    pub fn path(&self) -> &PathBuf {
        &self.path
    }
    /// `Arguments` value carrying config.axl overrides for this feature.
    pub fn overrides(&self) -> Value<'v> {
        self.overrides
    }
    pub fn frozen_handle(&self) -> Option<FrozenValue> {
        self.frozen_handle
    }

    /// Allocate a live `Feature` on `heap` from a `FrozenFeature` reference.
    ///
    /// Used by `MultiPhaseEval` to lift a feature out of its per-file frozen
    /// module and onto the shared heap, where config.axl can mutate it via
    /// `set_attr`. A fresh empty `Arguments` override store is allocated; the
    /// frozen-side overrides (always empty when called from Phase 1) are
    /// discarded.
    pub fn from_frozen(frozen_value: FrozenValue, heap: Heap<'v>) -> Self {
        let frozen = frozen_value
            .downcast_ref::<FrozenFeature>()
            .expect("from_frozen called with non-FrozenFeature value");
        let overrides = heap.alloc(Arguments::new());
        Feature {
            r#impl: frozen.r#impl.to_value(),
            args: frozen.args.clone(),
            summary: frozen.summary.clone(),
            description: frozen.description.clone(),
            display_name: RefCell::new(frozen.display_name.clone()),
            name: RefCell::new(frozen.name.clone()),
            export_name: RefCell::new(frozen.export_name.clone()),
            path: frozen.path.clone(),
            overrides,
            frozen_handle: Some(frozen_value),
        }
    }
}

impl<'v> FeatureLike<'v> for Feature<'v> {
    fn name(&self) -> String {
        self.name.borrow().clone()
    }
    fn export_name(&self) -> Option<String> {
        self.export_name.borrow().clone()
    }
    fn summary(&self) -> &String {
        &self.summary
    }
    fn description(&self) -> &String {
        &self.description
    }
    fn display_name(&self) -> String {
        self.display_name.borrow().clone()
    }
    fn args(&self) -> &SmallMap<String, Arg> {
        &self.args
    }
    fn path(&self) -> &PathBuf {
        &self.path
    }
    fn overrides(&self) -> Value<'v> {
        self.overrides
    }
    fn implementation(&self) -> Value<'v> {
        self.r#impl
    }
}

#[starlark_value(type = "feature")]
impl<'v> StarlarkValue<'v> for Feature<'v> {
    fn export_as(
        &self,
        variable_name: &str,
        _eval: &mut starlark::eval::Evaluator<'v, '_, '_>,
    ) -> starlark::Result<()> {
        validate_type_name(variable_name, "feature")
            .map_err(|e| starlark::Error::new_other(anyhow::anyhow!(e)))?;
        *self.export_name.borrow_mut() = Some(variable_name.to_string());
        let mut name = self.name.borrow_mut();
        if name.is_empty() {
            *name = to_command_name(variable_name);
        }
        let mut display_name = self.display_name.borrow_mut();
        if display_name.is_empty() {
            *display_name = camel_to_display_name(variable_name);
        }
        Ok(())
    }

    fn get_attr(&self, attribute: &str, heap: Heap<'v>) -> Option<Value<'v>> {
        match attribute {
            "args" => Some(self.overrides),
            "enabled" => Some(read_enabled(self.overrides, &self.args, heap)),
            _ => None,
        }
    }

    fn set_attr(&self, attribute: &str, value: Value<'v>) -> starlark::Result<()> {
        match attribute {
            "enabled" => {
                value.unpack_bool().ok_or_else(|| {
                    starlark::Error::new_other(anyhow::anyhow!(
                        "feature `enabled` must be a bool, got `{}`",
                        value.get_type()
                    ))
                })?;
                if let Some(args) = self.overrides.downcast_ref::<Arguments>() {
                    args.insert("enabled".to_owned(), value);
                }
                Ok(())
            }
            _ => Err(starlark::Error::new_other(anyhow::anyhow!(
                "feature attribute `{}` is read-only; use `.args.{} = ...` to set arg overrides",
                attribute,
                attribute
            ))),
        }
    }

    fn has_attr(&self, attribute: &str, _heap: Heap<'v>) -> bool {
        matches!(attribute, "args" | "enabled")
    }

    fn dir_attr(&self) -> Vec<String> {
        vec!["args".to_owned(), "enabled".to_owned()]
    }
}

/// Read the effective `enabled` value for a feature: stored override if present,
/// otherwise the schema default (`true` if absent).
fn read_enabled<'v>(
    overrides: Value<'v>,
    args_schema: &SmallMap<String, Arg>,
    heap: Heap<'v>,
) -> Value<'v> {
    if let Some(args) = overrides.downcast_ref::<Arguments>() {
        if let Some(v) = args.get("enabled") {
            return v;
        }
    }
    if let Some(args) = overrides.downcast_ref::<FrozenArguments>() {
        if let Some(fv) = args.get("enabled") {
            return fv.to_value();
        }
    }
    let default = matches!(
        args_schema.get("enabled"),
        Some(Arg::Boolean { default: true, .. })
    );
    heap.alloc(default)
}

impl<'v> values::AllocValue<'v> for Feature<'v> {
    fn alloc_value(self, heap: values::Heap<'v>) -> Value<'v> {
        heap.alloc_complex(self)
    }
}

impl<'v> values::Freeze for Feature<'v> {
    type Frozen = FrozenFeature;
    fn freeze(self, freezer: &values::Freezer) -> values::FreezeResult<Self::Frozen> {
        let frozen_impl = self.r#impl.freeze(freezer)?;
        Ok(FrozenFeature {
            r#impl: frozen_impl,
            args: self.args,
            summary: self.summary,
            description: self.description,
            display_name: self.display_name.into_inner(),
            name: self.name.into_inner(),
            export_name: self.export_name.into_inner(),
            path: self.path,
            overrides: self.overrides.freeze(freezer)?,
        })
    }
}

#[derive(Debug, Display, Clone, ProvidesStaticType, Trace, NoSerialize, Allocative)]
#[display("<Feature>")]
pub struct FrozenFeature {
    pub(super) r#impl: values::FrozenValue,
    #[allocative(skip)]
    pub(super) args: SmallMap<String, Arg>,
    pub(super) summary: String,
    pub(super) description: String,
    pub(super) display_name: String,
    pub(super) name: String,
    pub(super) export_name: Option<String>,
    pub(super) path: PathBuf,
    pub(super) overrides: values::FrozenValue,
}

starlark_simple_value!(FrozenFeature);

#[starlark_value(type = "feature")]
impl<'v> StarlarkValue<'v> for FrozenFeature {
    type Canonical = Feature<'v>;
}

impl FrozenFeature {
    pub fn implementation(&self) -> values::FrozenValue {
        self.r#impl
    }
    pub fn args(&self) -> &SmallMap<String, Arg> {
        &self.args
    }
    pub fn path(&self) -> &PathBuf {
        &self.path
    }
    pub fn overrides(&self) -> values::FrozenValue {
        self.overrides
    }
}

impl<'v> FeatureLike<'v> for FrozenFeature {
    fn name(&self) -> String {
        self.name.clone()
    }
    fn export_name(&self) -> Option<String> {
        self.export_name.clone()
    }
    fn summary(&self) -> &String {
        &self.summary
    }
    fn description(&self) -> &String {
        &self.description
    }
    fn display_name(&self) -> String {
        self.display_name.clone()
    }
    fn args(&self) -> &SmallMap<String, Arg> {
        &self.args
    }
    fn path(&self) -> &PathBuf {
        &self.path
    }
    fn overrides(&self) -> Value<'v> {
        self.overrides.to_value()
    }
    fn implementation(&self) -> Value<'v> {
        self.r#impl.to_value()
    }
}

struct FeatureImpl;

impl StarlarkCallableParamSpec for FeatureImpl {
    fn params() -> ParamSpec {
        ParamSpec::new_parts(
            [(
                ParamIsRequired::Yes,
                FeatureContext::get_type_starlark_repr(),
            )],
            [],
            None,
            [],
            None,
        )
        .unwrap()
    }
}

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
    /// `--artifact-upload:mode`. The name is auto-derived from the CamelCase export name
    /// via `to_command_name` (`ArtifactUpload` → `artifact-upload`) if not set explicitly.
    ///
    /// `display_name` overrides the Title Case name shown in CLI help section headings.
    ///
    /// ## Arg names
    ///
    /// Arg keys must be `snake_case` (`[a-z][a-z0-9_]*`). There are two kinds:
    ///
    /// - **CLI args** (`args.string(...)`, `args.boolean(...)`, etc.) — exposed as
    ///   `--{name}-{arg}` flags on every task subcommand; must be optional.
    /// - **Config-only args** (`args.custom(type, default = …)`) — set in `config.axl`
    ///   only, not shown in help.
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
    ///     summary = "Upload build artifacts to S3 storage",
    ///     implementation = _impl,
    ///     args = {
    ///         "bucket": args.custom(str | None, default = None),  # config.axl only
    ///         "mode":   args.string(default = "auto"),             # CLI flag: --artifact-upload-mode
    ///     },
    /// )
    /// ```
    fn feature<'v>(
        #[starlark(require = named)] implementation: values::typing::StarlarkCallable<
            'v,
            FeatureImpl,
            NoneType,
        >,
        #[starlark(require = named, default = String::new())] name: String,
        #[starlark(require = named, default = String::new())] display_name: String,
        #[starlark(require = named, default = String::new())] summary: String,
        #[starlark(require = named, default = String::new())] description: String,
        #[starlark(require = named, default = true)] enabled: bool,
        #[starlark(require = named, default = UnpackDictEntries::default())]
        args: UnpackDictEntries<String, Value<'v>>,
        eval: &mut starlark::eval::Evaluator<'v, '_, '_>,
    ) -> anyhow::Result<Feature<'v>> {
        let path = Env::current_script_path(eval)?;
        let overrides = eval.heap().alloc(Arguments::new());
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
                    "feature arg {:?}: expected args.string/boolean/... or args.custom(...), got '{}'",
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
                    "feature arg {:?}: CLI args in features must be optional (required = true \
                     is not allowed); features inject args into every task subcommand so required \
                     flags would break all tasks",
                    arg_name
                ));
            }
            args_.insert(arg_name, cli_arg.clone());
        }

        Ok(Feature {
            r#impl: implementation.0,
            args: args_,
            summary,
            description,
            display_name: RefCell::new(display_name),
            name: RefCell::new(name),
            export_name: RefCell::new(None),
            path,
            overrides,
            frozen_handle: None,
        })
    }
}
