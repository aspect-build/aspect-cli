use std::convert::Infallible;
use std::fmt::Display;
use std::u32;

use allocative::Allocative;

use starlark::environment::GlobalsBuilder;
use starlark::eval::Evaluator;
use starlark::starlark_module;
use starlark::starlark_simple_value;
use starlark::values::FrozenValue;
use starlark::values::NoSerialize;
use starlark::values::ProvidesStaticType;
use starlark::values::StarlarkValue;
use starlark::values::UnpackValue;
use starlark::values::Value;
use starlark::values::ValueLike;
use starlark::values::list::UnpackList;
use starlark::values::none::NoneOr;
use starlark::values::starlark_value;
use starlark::values::starlark_value_as_type::StarlarkValueAsType;

#[derive(Clone, Debug, ProvidesStaticType, NoSerialize, Allocative)]
pub enum Arg {
    String {
        required: bool,
        default: String,
        short: Option<String>,
        long: Option<String>,
        values: Option<Vec<String>>,
        description: Option<String>,
    },
    Boolean {
        required: bool,
        default: bool,
        short: Option<String>,
        long: Option<String>,
        description: Option<String>,
    },
    Int {
        required: bool,
        default: i32,
        short: Option<String>,
        long: Option<String>,
        description: Option<String>,
    },
    UInt {
        required: bool,
        default: u32,
        short: Option<String>,
        long: Option<String>,
        description: Option<String>,
    },
    Positional {
        minimum: u32,
        maximum: u32,
        default: Option<Vec<String>>,
        description: Option<String>,
    },
    TrailingVarArgs {
        description: Option<String>,
    },
    StringList {
        required: bool,
        default: Vec<String>,
        short: Option<String>,
        long: Option<String>,
        description: Option<String>,
    },
    BooleanList {
        required: bool,
        default: Vec<bool>,
        short: Option<String>,
        long: Option<String>,
        description: Option<String>,
    },
    IntList {
        required: bool,
        default: Vec<i32>,
        short: Option<String>,
        long: Option<String>,
        description: Option<String>,
    },
    UIntList {
        required: bool,
        default: Vec<u32>,
        short: Option<String>,
        long: Option<String>,
        description: Option<String>,
    },
    /// Config-only arg — not exposed on the CLI. Set via config.axl only.
    ///
    /// `typ_value` is `Some` when the type annotation is a frozen Starlark value
    /// (e.g. `str`, `int`, `bool`, `list[str]`). For parameterized types like
    /// `typing.Callable[[str], str]` that produce live values, it is `None` and
    /// type-checking is skipped at invocation time.
    Custom {
        #[allocative(skip)]
        typ_value: Option<FrozenValue>,
        #[allocative(skip)]
        default: Option<FrozenValue>,
        description: Option<String>,
    },
}

/// A CLI argument definition — the result of calling `args.string(...)`, `args.int(...)`, etc.
#[starlark_value(type = "args.Arg")]
impl<'v> StarlarkValue<'v> for Arg {}

starlark_simple_value!(Arg);

impl Arg {
    /// Returns `true` if this arg was declared with `required = true`.
    ///
    /// Positional, TrailingVarArgs, and Custom do not carry a `required` field — callers
    /// should disallow those in contexts where required args are not acceptable.
    pub fn is_required(&self) -> bool {
        match self {
            Self::String { required, .. }
            | Self::Boolean { required, .. }
            | Self::Int { required, .. }
            | Self::UInt { required, .. }
            | Self::StringList { required, .. }
            | Self::BooleanList { required, .. }
            | Self::IntList { required, .. }
            | Self::UIntList { required, .. } => *required,
            Self::Positional { .. } | Self::TrailingVarArgs { .. } | Self::Custom { .. } => false,
        }
    }

    /// Returns `true` if this arg is exposed on the CLI (flags, positional, or trailing).
    /// Only `Custom` is not CLI-exposed (config.axl only).
    pub fn is_cli_exposed(&self) -> bool {
        !matches!(self, Self::Custom { .. })
    }

    /// Returns the `long` override if set, otherwise `None`.
    pub fn long_override(&self) -> Option<&str> {
        match self {
            Self::String { long, .. }
            | Self::Boolean { long, .. }
            | Self::Int { long, .. }
            | Self::UInt { long, .. }
            | Self::StringList { long, .. }
            | Self::BooleanList { long, .. }
            | Self::IntList { long, .. }
            | Self::UIntList { long, .. } => long.as_deref(),
            Self::Positional { .. } | Self::TrailingVarArgs { .. } | Self::Custom { .. } => None,
        }
    }
}

impl Display for Arg {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::String { .. } => write!(f, "<args.Arg: string>"),
            Self::Boolean { .. } => write!(f, "<args.Arg: boolean>"),
            Self::Int { .. } => write!(f, "<args.Arg: int>"),
            Self::UInt { .. } => write!(f, "<args.Arg: uint>"),
            Self::Positional { .. } => write!(f, "<args.Arg: positional>"),
            Self::TrailingVarArgs { .. } => {
                write!(f, "<args.Arg: trailing variable arguments>")
            }
            Self::StringList { .. } => write!(f, "<args.Arg: string_list>"),
            Self::BooleanList { .. } => write!(f, "<args.Arg: boolean_list>"),
            Self::IntList { .. } => write!(f, "<args.Arg: int_list>"),
            Self::UIntList { .. } => write!(f, "<args.Arg: uint_list>"),
            Self::Custom { .. } => write!(f, "<args.Arg: custom>"),
        }
    }
}

impl<'v> UnpackValue<'v> for Arg {
    type Error = Infallible;

    fn unpack_value_impl(value: Value<'v>) -> Result<Option<Self>, Self::Error> {
        Ok(value.downcast_ref::<Self>().map(|value| value.clone()))
    }
}

/// Validate and unwrap the `long` override into `Option<String>`.
///
/// Accepts `[a-z][a-z0-9_-]*(:[a-z][a-z0-9_-]*)?`: one or two lowercase
/// kebab/snake segments separated by at most one colon. The colon form
/// (`feature-name:flag-name`) is used by feature args to carry the namespace;
/// task args reject it at task definition time.
fn validated_long(long: NoneOr<String>) -> starlark::Result<Option<String>> {
    if let NoneOr::Other(ref s) = long {
        fn valid_segment(seg: &str) -> bool {
            let mut chars = seg.chars();
            matches!(chars.next(), Some(c) if c.is_ascii_lowercase())
                && chars
                    .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-' || c == '_')
        }
        let valid = match s.split_once(':') {
            None => valid_segment(s),
            Some((prefix, suffix)) => valid_segment(prefix) && valid_segment(suffix),
        };
        if !valid {
            return Err(starlark::Error::new_kind(starlark::ErrorKind::Function(
                anyhow::anyhow!(
                    "`long` must match [a-z][a-z0-9_-]*(:[a-z][a-z0-9_-]*)?; got {:?}",
                    s
                ),
            )));
        }
    }
    Ok(long.into_option())
}

#[starlark_module]
pub fn register_globals(globals: &mut GlobalsBuilder) {
    const Args: StarlarkValueAsType<Arg> = StarlarkValueAsType::new();

    /// Defines a positional argument that accepts a range of values.
    fn positional<'v>(
        #[starlark(require = named, default = 0)] minimum: u32,
        #[starlark(require = named, default = 1)] maximum: u32,
        #[starlark(require = named, default = NoneOr::None)] default: NoneOr<UnpackList<String>>,
        #[starlark(require = named, default = NoneOr::None)] description: NoneOr<String>,
    ) -> anyhow::Result<Arg> {
        Ok(Arg::Positional {
            minimum,
            maximum,
            default: default.into_option().map(|it| it.items),
            description: description.into_option(),
        })
    }

    /// Defines a trailing variable argument that captures the remaining arguments without further parsing.
    /// Only one such argument is permitted, and it must be the last in the sequence.
    fn trailing_var_args<'v>(
        #[starlark(require = named, default = NoneOr::None)] description: NoneOr<String>,
    ) -> anyhow::Result<Arg> {
        Ok(Arg::TrailingVarArgs {
            description: description.into_option(),
        })
    }

    /// Defines a string flag that can be specified as `--flag_name=flag_value`.
    ///
    /// Use `long = "override-name"` to override the default kebab-case derivation.
    fn string<'v>(
        #[starlark(require = named, default = false)] required: bool,
        #[starlark(require = named)] default: Option<String>,
        #[starlark(require = named, default = NoneOr::None)] short: NoneOr<String>,
        #[starlark(require = named, default = NoneOr::None)] long: NoneOr<String>,
        #[starlark(require = named, default = NoneOr::None)] values: NoneOr<UnpackList<String>>,
        #[starlark(require = named, default = NoneOr::None)] description: NoneOr<String>,
    ) -> starlark::Result<Arg> {
        if required && default.is_some() {
            return Err(starlark::Error::new_kind(starlark::ErrorKind::Function(
                anyhow::anyhow!("`required` and `default` are both set."),
            )));
        }
        if matches!(short, NoneOr::Other(ref s) if s.len() != 1) {
            return Err(starlark::Error::new_kind(starlark::ErrorKind::Function(
                anyhow::anyhow!("`short` must be a 1-character string."),
            )));
        }
        Ok(Arg::String {
            required,
            default: default.unwrap_or_default(),
            short: short.into_option(),
            long: validated_long(long)?,
            values: values.into_option().map(|it| it.items),
            description: description.into_option(),
        })
    }

    /// Defines a string list flag that can be specified multiple times.
    ///
    /// Use `long = "override-name"` to override the default kebab-case derivation.
    fn string_list<'v>(
        #[starlark(require = named, default = false)] required: bool,
        #[starlark(require = named, default = NoneOr::None)] default: NoneOr<UnpackList<String>>,
        #[starlark(require = named, default = NoneOr::None)] short: NoneOr<String>,
        #[starlark(require = named, default = NoneOr::None)] long: NoneOr<String>,
        #[starlark(require = named, default = NoneOr::None)] description: NoneOr<String>,
    ) -> starlark::Result<Arg> {
        if required && !default.is_none() {
            return Err(starlark::Error::new_kind(starlark::ErrorKind::Function(
                anyhow::anyhow!("`required` and `default` are both set."),
            )));
        }
        if matches!(short, NoneOr::Other(ref s) if s.len() != 1) {
            return Err(starlark::Error::new_kind(starlark::ErrorKind::Function(
                anyhow::anyhow!("`short` must be a 1-character string."),
            )));
        }
        Ok(Arg::StringList {
            required,
            default: default.into_option().map(|it| it.items).unwrap_or_default(),
            short: short.into_option(),
            long: validated_long(long)?,
            description: description.into_option(),
        })
    }

    /// Defines a boolean flag. Use `--flag_name` (true) or `--flag_name=false`.
    ///
    /// Use `long = "override-name"` to override the default kebab-case derivation.
    fn boolean<'v>(
        #[starlark(require = named, default = false)] required: bool,
        #[starlark(require = named)] default: Option<bool>,
        #[starlark(require = named, default = NoneOr::None)] short: NoneOr<String>,
        #[starlark(require = named, default = NoneOr::None)] long: NoneOr<String>,
        #[starlark(require = named, default = NoneOr::None)] description: NoneOr<String>,
        _eval: &mut Evaluator<'v, '_, '_>,
    ) -> starlark::Result<Arg> {
        if required && default.is_some() {
            return Err(starlark::Error::new_kind(starlark::ErrorKind::Function(
                anyhow::anyhow!("`required` and `default` are both set."),
            )));
        }
        if matches!(short, NoneOr::Other(ref s) if s.len() != 1) {
            return Err(starlark::Error::new_kind(starlark::ErrorKind::Function(
                anyhow::anyhow!("`short` must be a 1-character string."),
            )));
        }
        Ok(Arg::Boolean {
            required,
            default: default.unwrap_or_default(),
            short: short.into_option(),
            long: validated_long(long)?,
            description: description.into_option(),
        })
    }

    /// Defines a boolean list flag that can be specified multiple times.
    ///
    /// Use `long = "override-name"` to override the default kebab-case derivation.
    fn boolean_list<'v>(
        #[starlark(require = named, default = false)] required: bool,
        #[starlark(require = named, default = NoneOr::None)] default: NoneOr<UnpackList<bool>>,
        #[starlark(require = named, default = NoneOr::None)] short: NoneOr<String>,
        #[starlark(require = named, default = NoneOr::None)] long: NoneOr<String>,
        #[starlark(require = named, default = NoneOr::None)] description: NoneOr<String>,
    ) -> starlark::Result<Arg> {
        if required && !default.is_none() {
            return Err(starlark::Error::new_kind(starlark::ErrorKind::Function(
                anyhow::anyhow!("`required` and `default` are both set."),
            )));
        }
        if matches!(short, NoneOr::Other(ref s) if s.len() != 1) {
            return Err(starlark::Error::new_kind(starlark::ErrorKind::Function(
                anyhow::anyhow!("`short` must be a 1-character string."),
            )));
        }
        Ok(Arg::BooleanList {
            required,
            default: default.into_option().map(|it| it.items).unwrap_or_default(),
            short: short.into_option(),
            long: validated_long(long)?,
            description: description.into_option(),
        })
    }

    /// Defines an integer flag.
    ///
    /// Use `long = "override-name"` to override the default kebab-case derivation.
    fn int<'v>(
        #[starlark(require = named, default = false)] required: bool,
        #[starlark(require = named)] default: Option<i32>,
        #[starlark(require = named, default = NoneOr::None)] short: NoneOr<String>,
        #[starlark(require = named, default = NoneOr::None)] long: NoneOr<String>,
        #[starlark(require = named, default = NoneOr::None)] description: NoneOr<String>,
    ) -> starlark::Result<Arg> {
        if required && default.is_some() {
            return Err(starlark::Error::new_kind(starlark::ErrorKind::Function(
                anyhow::anyhow!("`required` and `default` are both set."),
            )));
        }
        if matches!(short, NoneOr::Other(ref s) if s.len() != 1) {
            return Err(starlark::Error::new_kind(starlark::ErrorKind::Function(
                anyhow::anyhow!("`short` must be a 1-character string."),
            )));
        }
        Ok(Arg::Int {
            required,
            default: default.unwrap_or_default(),
            short: short.into_option(),
            long: validated_long(long)?,
            description: description.into_option(),
        })
    }

    /// Defines an integer list flag that can be specified multiple times.
    ///
    /// Use `long = "override-name"` to override the default kebab-case derivation.
    fn int_list<'v>(
        #[starlark(require = named, default = false)] required: bool,
        #[starlark(require = named, default = NoneOr::None)] default: NoneOr<UnpackList<i32>>,
        #[starlark(require = named, default = NoneOr::None)] short: NoneOr<String>,
        #[starlark(require = named, default = NoneOr::None)] long: NoneOr<String>,
        #[starlark(require = named, default = NoneOr::None)] description: NoneOr<String>,
    ) -> starlark::Result<Arg> {
        if required && !default.is_none() {
            return Err(starlark::Error::new_kind(starlark::ErrorKind::Function(
                anyhow::anyhow!("`required` and `default` are both set."),
            )));
        }
        if matches!(short, NoneOr::Other(ref s) if s.len() != 1) {
            return Err(starlark::Error::new_kind(starlark::ErrorKind::Function(
                anyhow::anyhow!("`short` must be a 1-character string."),
            )));
        }
        Ok(Arg::IntList {
            required,
            default: default.into_option().map(|it| it.items).unwrap_or_default(),
            short: short.into_option(),
            long: validated_long(long)?,
            description: description.into_option(),
        })
    }

    /// Defines an unsigned integer flag.
    ///
    /// Use `long = "override-name"` to override the default kebab-case derivation.
    fn uint<'v>(
        #[starlark(require = named, default = false)] required: bool,
        #[starlark(require = named)] default: Option<u32>,
        #[starlark(require = named, default = NoneOr::None)] short: NoneOr<String>,
        #[starlark(require = named, default = NoneOr::None)] long: NoneOr<String>,
        #[starlark(require = named, default = NoneOr::None)] description: NoneOr<String>,
    ) -> starlark::Result<Arg> {
        if required && default.is_some() {
            return Err(starlark::Error::new_kind(starlark::ErrorKind::Function(
                anyhow::anyhow!("`required` and `default` are both set."),
            )));
        }
        if matches!(short, NoneOr::Other(ref s) if s.len() != 1) {
            return Err(starlark::Error::new_kind(starlark::ErrorKind::Function(
                anyhow::anyhow!("`short` must be a 1-character string."),
            )));
        }
        Ok(Arg::UInt {
            required,
            default: default.unwrap_or_default(),
            short: short.into_option(),
            long: validated_long(long)?,
            description: description.into_option(),
        })
    }

    /// Defines an unsigned integer list flag that can be specified multiple times.
    ///
    /// Use `long = "override-name"` to override the default kebab-case derivation.
    fn uint_list<'v>(
        #[starlark(require = named, default = false)] required: bool,
        #[starlark(require = named, default = NoneOr::None)] default: NoneOr<UnpackList<u32>>,
        #[starlark(require = named, default = NoneOr::None)] short: NoneOr<String>,
        #[starlark(require = named, default = NoneOr::None)] long: NoneOr<String>,
        #[starlark(require = named, default = NoneOr::None)] description: NoneOr<String>,
    ) -> starlark::Result<Arg> {
        if required && !default.is_none() {
            return Err(starlark::Error::new_kind(starlark::ErrorKind::Function(
                anyhow::anyhow!("`required` and `default` are both set."),
            )));
        }
        if matches!(short, NoneOr::Other(ref s) if s.len() != 1) {
            return Err(starlark::Error::new_kind(starlark::ErrorKind::Function(
                anyhow::anyhow!("`short` must be a 1-character string."),
            )));
        }
        Ok(Arg::UIntList {
            required,
            default: default.into_option().map(|it| it.items).unwrap_or_default(),
            short: short.into_option(),
            long: validated_long(long)?,
            description: description.into_option(),
        })
    }

    /// Defines a config-only arg — not exposed on the CLI. Set via config.axl only.
    ///
    /// The `type` argument must be a built-in or otherwise frozen type (e.g. `str`, `int`,
    /// `bool`, `list[str]`). If provided, `default` must match the declared type.
    ///
    /// Example:
    /// ```starlark
    /// my_task = task(
    ///     implementation = _impl,
    ///     args = {
    ///         "mode": args.string(default = "auto"),
    ///         "bucket": args.custom(str | None, default = None),  # config.axl only
    ///     },
    /// )
    /// ```
    fn custom<'v>(
        #[starlark(require = pos)] typ: Value<'v>,
        #[starlark(require = named)] default: Option<Value<'v>>,
        #[starlark(require = named, default = NoneOr::None)] description: NoneOr<String>,
        eval: &mut Evaluator<'v, '_, '_>,
    ) -> anyhow::Result<Arg> {
        // Try to store the type as a frozen value (enables type-checking at invoke time).
        // Parameterized types like `typing.Callable[[str], str]` are live values and cannot
        // be frozen here — in that case we store None and skip runtime type-checking.
        let typ_value = typ.unpack_frozen();

        // Always build TypeCompiled from the live value so we can validate the default.
        let compiled = starlark::values::typing::TypeCompiled::new(typ, eval.heap())
            .map_err(|e| anyhow::anyhow!("{:?}", e))?;

        // Validate and freeze the default. Live values (e.g. lambdas defined inline) cannot be
        // stored in Arg which requires FrozenValue; silently store None in that case.
        // Type validation still runs when the type is compilable, so mismatches are caught.
        //
        // KNOWN LIMITATION: live container literals (`{}`, `[]`) are also not yet frozen here,
        // so `args.custom(dict, default = {})` ends up with `default = None` at access time.
        // Callers must defensively `or {}` / `or []`. A proper fix needs to deep-freeze
        // freezable values (dict/list/string/int/bool/tuple) into the frozen heap before
        // storing.
        let default_frozen = match default {
            None => None,
            Some(d) => {
                if !compiled.matches(d) {
                    return Err(anyhow::anyhow!(
                        "args.custom() default `{}` does not match type `{}`",
                        d,
                        compiled
                    ));
                }
                // Live values (e.g. inline lambdas, fresh dict/list literals) cannot be frozen
                // here — store None.
                d.unpack_frozen()
            }
        };

        Ok(Arg::Custom {
            typ_value,
            default: default_frozen,
            description: description.into_option(),
        })
    }
}
