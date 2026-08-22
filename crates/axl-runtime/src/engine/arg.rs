use std::convert::Infallible;
use std::fmt::Display;
use std::u32;

use allocative::Allocative;

use starlark::environment::GlobalsBuilder;
use starlark::eval::Evaluator;
use starlark::starlark_module;
use starlark::starlark_simple_value;
use starlark::values::FrozenValue;
use starlark::values::Heap;
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
use starlark::values::typing::TypeCompiled;

/// Which side of the task name an [`Arg::Passthrough`] collects from.
///
/// The split mirrors Bazel's own command line, where an option's meaning
/// depends on whether it precedes the command: `bazel --output_base=/tmp build`
/// vs `bazel build --keep_going`. A task wrapping another tool declares one
/// bucket per slot so it can forward each to the right place.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Allocative)]
pub enum PassthroughPosition {
    /// Flags typed before the task name (Bazel's startup-option slot).
    PreCommand,
    /// Flags typed after the task name.
    PostCommand,
}

impl PassthroughPosition {
    pub const PRE_COMMAND: &'static str = "pre_command";
    pub const POST_COMMAND: &'static str = "post_command";

    pub fn parse(s: &str) -> Option<Self> {
        match s {
            Self::PRE_COMMAND => Some(Self::PreCommand),
            Self::POST_COMMAND => Some(Self::PostCommand),
            _ => None,
        }
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Self::PreCommand => Self::PRE_COMMAND,
            Self::PostCommand => Self::POST_COMMAND,
        }
    }
}

#[derive(Clone, Debug, ProvidesStaticType, NoSerialize, Allocative)]
pub enum Arg {
    String {
        required: bool,
        default: String,
        short: Option<String>,
        long: Option<String>,
        values: Option<Vec<String>>,
        description: Option<String>,
        /// Value to use when the flag is given with no `=value`, making a bare
        /// `--flag` legal (`args.string(bare = "…")`). `None` keeps the default
        /// behavior, where the flag requires a value. Distinct from `default`,
        /// which applies when the flag is absent entirely — a flag with both can
        /// tell "not passed" from "passed bare".
        bare: Option<String>,
        /// Keep this arg off the CLI: settable from `config.axl` only. See
        /// [`Arg::config_only`].
        config_only: bool,
    },
    Boolean {
        required: bool,
        default: bool,
        short: Option<String>,
        long: Option<String>,
        description: Option<String>,
        /// Keep this arg off the CLI: settable from `config.axl` only. See
        /// [`Arg::config_only`].
        config_only: bool,
    },
    Int {
        required: bool,
        default: i32,
        short: Option<String>,
        long: Option<String>,
        description: Option<String>,
        /// Keep this arg off the CLI: settable from `config.axl` only. See
        /// [`Arg::config_only`].
        config_only: bool,
    },
    UInt {
        required: bool,
        default: u32,
        short: Option<String>,
        long: Option<String>,
        description: Option<String>,
        /// Keep this arg off the CLI: settable from `config.axl` only. See
        /// [`Arg::config_only`].
        config_only: bool,
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
    /// Collects the hyphen-led tokens the task does not otherwise declare, in
    /// command-line order, instead of failing the parse with "unexpected
    /// argument". `position` selects which side of the task name it collects
    /// from; a task may declare one bucket per position.
    ///
    /// Tokens are collected verbatim: nothing is rewritten, so the wrapped tool
    /// sees what was typed. A collected flag claims the token after it only when
    /// `value_flags_from` lists it as taking a separate value — the arity of a
    /// flag nobody declared is otherwise unknowable, so `--unknown 8` collects
    /// the flag and leaves `8` as a positional.
    ///
    /// Declaring a bucket comes with an obligation: the task must claim it
    /// (`ctx.args.claim(name)`) to say those flags are its to act on. A run that
    /// leaves collected flags unclaimed is failed by the runtime rather than
    /// dropping them silently.
    Passthrough {
        position: PassthroughPosition,
        /// Name of a sibling `args.string_list` arg listing the flags that take
        /// a separate value (`-c opt`), so those two tokens are collected
        /// together instead of the value being left behind as a positional.
        /// Reading it from a sibling arg rather than inline is what lets
        /// `config.axl` replace the list.
        value_flags_from: Option<String>,
        description: Option<String>,
    },
    StringList {
        required: bool,
        default: Vec<String>,
        short: Option<String>,
        long: Option<String>,
        description: Option<String>,
        /// Keep this arg off the CLI: settable from `config.axl` only. See
        /// [`Arg::config_only`].
        config_only: bool,
    },
    BooleanList {
        required: bool,
        default: Vec<bool>,
        short: Option<String>,
        long: Option<String>,
        description: Option<String>,
        /// Keep this arg off the CLI: settable from `config.axl` only. See
        /// [`Arg::config_only`].
        config_only: bool,
    },
    IntList {
        required: bool,
        default: Vec<i32>,
        short: Option<String>,
        long: Option<String>,
        description: Option<String>,
        /// Keep this arg off the CLI: settable from `config.axl` only. See
        /// [`Arg::config_only`].
        config_only: bool,
    },
    UIntList {
        required: bool,
        default: Vec<u32>,
        short: Option<String>,
        long: Option<String>,
        description: Option<String>,
        /// Keep this arg off the CLI: settable from `config.axl` only. See
        /// [`Arg::config_only`].
        config_only: bool,
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
    /// Positional, TrailingVarArgs, Passthrough, and Custom do not carry a `required`
    /// field — callers should disallow those in contexts where required args are not
    /// acceptable.
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
            Self::Positional { .. }
            | Self::TrailingVarArgs { .. }
            | Self::Passthrough { .. }
            | Self::Custom { .. } => false,
        }
    }

    /// Returns `true` if this arg was declared `config_only = True`, keeping it
    /// off the CLI and settable from config.axl only.
    ///
    /// Every flag-shaped kind carries the knob. The argv-shaped kinds
    /// (`positional`, `trailing_var_args`, `passthrough`) do not: their content
    /// comes from the command line, so there would be nothing left to set. See
    /// also [`Arg::Custom`], which is config-only by construction.
    pub fn config_only(&self) -> bool {
        match self {
            Self::String { config_only, .. }
            | Self::Boolean { config_only, .. }
            | Self::Int { config_only, .. }
            | Self::UInt { config_only, .. }
            | Self::StringList { config_only, .. }
            | Self::BooleanList { config_only, .. }
            | Self::IntList { config_only, .. }
            | Self::UIntList { config_only, .. } => *config_only,
            Self::Positional { .. }
            | Self::TrailingVarArgs { .. }
            | Self::Passthrough { .. }
            | Self::Custom { .. } => false,
        }
    }

    /// Returns `true` if this arg is exposed on the CLI (flags, positional, or
    /// trailing). `Custom` never is, and any flag declared
    /// `config_only = True` opts out — all are set from config.axl only.
    pub fn is_cli_exposed(&self) -> bool {
        !matches!(self, Self::Custom { .. }) && !self.config_only()
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
            Self::Positional { .. }
            | Self::TrailingVarArgs { .. }
            | Self::Passthrough { .. }
            | Self::Custom { .. } => None,
        }
    }

    /// The `description = "…"` this arg was declared with, if any.
    pub fn description(&self) -> Option<&str> {
        match self {
            Self::String { description, .. }
            | Self::Boolean { description, .. }
            | Self::Int { description, .. }
            | Self::UInt { description, .. }
            | Self::Positional { description, .. }
            | Self::TrailingVarArgs { description }
            | Self::Passthrough { description, .. }
            | Self::StringList { description, .. }
            | Self::BooleanList { description, .. }
            | Self::IntList { description, .. }
            | Self::UIntList { description, .. }
            | Self::Custom { description, .. } => description.as_deref(),
        }
    }

    /// The position an `args.passthrough()` arg collects from, or `None` for
    /// every other kind.
    pub fn passthrough_position(&self) -> Option<PassthroughPosition> {
        match self {
            Self::Passthrough { position, .. } => Some(*position),
            _ => None,
        }
    }

    /// The sibling arg an `args.passthrough()` reads its value-taking flag list
    /// from, if it declares one.
    pub fn value_flags_from(&self) -> Option<&str> {
        match self {
            Self::Passthrough {
                value_flags_from, ..
            } => value_flags_from.as_deref(),
            _ => None,
        }
    }

    /// This arg's declared list default, for the kinds that carry one. The
    /// schema-level value of a `config_only` list before any config.axl
    /// override is layered on.
    pub fn string_list_default(&self) -> Option<&[String]> {
        match self {
            Self::StringList { default, .. } => Some(default),
            _ => None,
        }
    }

    /// Return a clone of `self` with this variant's `default` replaced by `value`.
    ///
    /// Used by `task.alias(defaults = {...})` to overlay new defaults on a base
    /// task's arg schema. `value`'s Starlark type must match the variant — a
    /// mismatch produces an error mentioning `arg_name`. The schema constraints
    /// declared on the original arg are re-enforced: `args.string(values =
    /// [...])` membership and `args.custom(type, ...)` type predicates.
    ///
    /// `TrailingVarArgs` carries no schema-level default and rejects override
    /// attempts. `Custom` defaults that cannot be frozen (live containers,
    /// inline lambdas) drop to `None` — the same limitation `args.custom(...)`
    /// has at definition time.
    pub fn with_default<'v>(
        &self,
        arg_name: &str,
        value: Value<'v>,
        heap: Heap<'v>,
    ) -> anyhow::Result<Arg> {
        let mut next = self.clone();
        match &mut next {
            Self::String {
                default, values, ..
            } => {
                *default = unpack_typed::<String>(arg_name, "string", value)?;
                if let Some(allowed) = values
                    && !allowed.iter().any(|v| v == default)
                {
                    return Err(anyhow::anyhow!(
                        "arg {:?}: value {:?} is not one of the allowed values {:?}",
                        arg_name,
                        default,
                        allowed,
                    ));
                }
            }
            Self::Boolean { default, .. } => {
                *default = unpack_typed::<bool>(arg_name, "boolean", value)?;
            }
            Self::Int { default, .. } => {
                *default = unpack_typed::<i32>(arg_name, "int", value)?;
            }
            Self::UInt { default, .. } => {
                *default = unpack_typed::<u32>(arg_name, "uint", value)?;
            }
            Self::Positional { default, .. } => {
                *default = Some(unpack_list_items::<String>(arg_name, "positional", value)?);
            }
            Self::StringList { default, .. } => {
                *default = unpack_list_items::<String>(arg_name, "string_list", value)?;
            }
            Self::BooleanList { default, .. } => {
                *default = unpack_list_items::<bool>(arg_name, "boolean_list", value)?;
            }
            Self::IntList { default, .. } => {
                *default = unpack_list_items::<i32>(arg_name, "int_list", value)?;
            }
            Self::UIntList { default, .. } => {
                *default = unpack_list_items::<u32>(arg_name, "uint_list", value)?;
            }
            Self::Custom {
                typ_value, default, ..
            } => {
                if let Some(typ) = typ_value {
                    let compiled = TypeCompiled::new(typ.to_value(), heap)
                        .map_err(|e| anyhow::anyhow!("{:?}", e))?;
                    if !compiled.matches(value) {
                        return Err(anyhow::anyhow!(
                            "arg {:?}: value `{}` does not match arg type `{}`",
                            arg_name,
                            value,
                            compiled,
                        ));
                    }
                }
                *default = value.unpack_frozen();
            }
            Self::TrailingVarArgs { .. } => {
                return Err(anyhow::anyhow!(
                    "arg {:?}: cannot override a trailing_var_args default — \
                     the schema does not carry one",
                    arg_name,
                ));
            }
            Self::Passthrough { .. } => {
                return Err(anyhow::anyhow!(
                    "arg {:?}: cannot override a passthrough default — it holds \
                     whatever the command line was not otherwise able to parse",
                    arg_name,
                ));
            }
        }
        Ok(next)
    }
}

fn type_error(arg_name: &str, expected: &str, got: Value<'_>) -> anyhow::Error {
    anyhow::anyhow!(
        "arg {:?}: expected {}, got '{}'",
        arg_name,
        expected,
        got.get_type(),
    )
}

fn unpack_typed<'v, T: UnpackValue<'v>>(
    arg_name: &str,
    expected: &str,
    value: Value<'v>,
) -> anyhow::Result<T> {
    T::unpack_value(value)
        .ok()
        .flatten()
        .ok_or_else(|| type_error(arg_name, expected, value))
}

fn unpack_list_items<'v, T: UnpackValue<'v>>(
    arg_name: &str,
    expected: &str,
    value: Value<'v>,
) -> anyhow::Result<Vec<T>> {
    UnpackList::<T>::unpack_value(value)
        .ok()
        .flatten()
        .map(|l| l.items)
        .ok_or_else(|| type_error(arg_name, expected, value))
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
            Self::Passthrough { position, .. } => {
                write!(f, "<args.Arg: passthrough {}>", position.as_str())
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

/// Reject `config_only` alongside a knob that names or demands a command-line
/// spelling, which a config-only arg does not have.
fn validated_config_only(
    config_only: bool,
    required: bool,
    short: &NoneOr<String>,
    long: &NoneOr<String>,
) -> starlark::Result<bool> {
    if config_only && (required || !short.is_none() || !long.is_none()) {
        return Err(starlark::Error::new_kind(starlark::ErrorKind::Function(
            anyhow::anyhow!(
                "`config_only` cannot be combined with `required`, `short` or `long` — \
                 the arg has no command-line spelling to require or name."
            ),
        )));
    }
    Ok(config_only)
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

// A Starlark constructor's kwargs become positional parameters of the generated
// function, so the arg-count lint fires on `args.string` (8 named kwargs).
#[allow(clippy::too_many_arguments)]
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

    /// Defines a bucket that collects the flags this task does not declare,
    /// instead of failing the parse with "unexpected argument". For a task that
    /// wraps another tool (`bazel`, `cargo`, …) and wants that tool's own flags
    /// to work without being individually redeclared or wrapped.
    ///
    /// `position` selects which side of the task name a flag is collected from:
    ///
    /// * `"pre_command"` — before it: `aspect --output_base=/tmp build`. This is
    ///   where Bazel takes its startup options.
    /// * `"post_command"` — after it: `aspect build --remote_download_all`.
    ///
    /// A task may declare one bucket per position; both are lists of strings in
    /// command-line order. Keeping them separate is what lets a task forward
    /// each set to the slot it was typed for.
    ///
    /// `value_flags_from` names a sibling `args.string_list` of the flags that
    /// take a separate value, so `-c opt` is collected as a pair. Without it — or
    /// for a flag the list omits — the token after a collected flag is left
    /// alone, since the arity of a flag nobody declared is unknowable: `--jobs 8`
    /// would collect `--jobs` and leave `8` as a positional. Keeping the list in
    /// a sibling arg rather than inline is what lets `config.axl` replace it.
    ///
    /// Nothing is rewritten on the way through: what the wrapped tool receives
    /// is what was typed.
    ///
    /// **Read a bucket with `ctx.args.claim(name)`**, which both returns the
    /// list and tells the runtime this task is responsible for those flags.
    /// Buying out of the "unexpected argument" error is only sound if something
    /// acts on what was collected, so a run that ends with flags in an
    /// unclaimed bucket fails — reading the value as a plain attribute is
    /// inspecting, not forwarding. A path that deliberately runs nothing can
    /// claim to say so.
    ///
    /// Example:
    /// ```starlark
    /// def _impl(ctx):
    ///     tool_flags = ctx.args.claim("tool_flags")
    ///     return ctx.std.process.command("some-tool").args(tool_flags).spawn().wait().code
    ///
    /// my_task = task(
    ///     implementation = _impl,
    ///     args = {
    ///         "targets": args.positional(minimum = 1, maximum = 512),
    ///         "tool_flags": args.passthrough(position = "post_command"),
    ///     },
    /// )
    /// ```
    fn passthrough<'v>(
        #[starlark(require = named)] position: String,
        #[starlark(require = named, default = NoneOr::None)] value_flags_from: NoneOr<String>,
        #[starlark(require = named, default = NoneOr::None)] description: NoneOr<String>,
    ) -> starlark::Result<Arg> {
        let Some(position) = PassthroughPosition::parse(&position) else {
            return Err(starlark::Error::new_kind(starlark::ErrorKind::Function(
                anyhow::anyhow!(
                    "`position` must be {:?} or {:?}; got {:?}",
                    PassthroughPosition::PRE_COMMAND,
                    PassthroughPosition::POST_COMMAND,
                    position,
                ),
            )));
        };
        Ok(Arg::Passthrough {
            position,
            value_flags_from: value_flags_from.into_option(),
            description: description.into_option(),
        })
    }

    /// Defines a string flag that can be specified as `--flag_name=flag_value`.
    ///
    /// Use `long = "override-name"` to override the default kebab-case derivation.
    ///
    /// `config_only = True` keeps the arg off the command line entirely: it is
    /// then settable only from `config.axl`, for a value a task reads but nobody
    /// should type. Every flag-shaped constructor takes it, and it cannot be
    /// combined with `required`, `short` or `long`, which name or demand a
    /// command-line spelling that does not exist. `args.custom(type, ...)` is
    /// config-only by construction and so has no such parameter; the argv-shaped
    /// kinds (`positional`, `trailing_var_args`, `passthrough`) have none either,
    /// since their content comes from the command line.
    ///
    /// `bare = "value"` additionally makes a valueless `--flag_name` legal,
    /// resolving to `value` — for a flag whose common case needs no argument
    /// (`--remote` meaning "the usual capabilities"). Without it, omitting the
    /// value is an error. `bare` and `default` are independent, so a flag can
    /// distinguish "absent" from "passed with no value".
    fn string<'v>(
        #[starlark(require = named, default = false)] required: bool,
        #[starlark(require = named)] default: Option<String>,
        #[starlark(require = named, default = NoneOr::None)] short: NoneOr<String>,
        #[starlark(require = named, default = NoneOr::None)] long: NoneOr<String>,
        #[starlark(require = named, default = NoneOr::None)] values: NoneOr<UnpackList<String>>,
        #[starlark(require = named, default = NoneOr::None)] description: NoneOr<String>,
        #[starlark(require = named, default = NoneOr::None)] bare: NoneOr<String>,
        #[starlark(require = named, default = false)] config_only: bool,
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
        let bare = bare.into_option();
        // A `values` list constrains what the flag accepts, so a `bare` outside it
        // would be unreachable-but-injected — reject it at declaration instead.
        if let (Some(bare), NoneOr::Other(allowed)) = (bare.as_ref(), &values) {
            if !allowed.items.contains(bare) {
                return Err(starlark::Error::new_kind(starlark::ErrorKind::Function(
                    anyhow::anyhow!("`bare` value {bare:?} is not among the allowed `values`."),
                )));
            }
        }
        let config_only = validated_config_only(config_only, required, &short, &long)?;
        Ok(Arg::String {
            required,
            default: default.unwrap_or_default(),
            short: short.into_option(),
            long: validated_long(long)?,
            values: values.into_option().map(|it| it.items),
            description: description.into_option(),
            bare,
            config_only,
        })
    }

    /// Defines a string list flag that can be specified multiple times.
    ///
    /// Use `long = "override-name"` to override the default kebab-case derivation.
    /// `config_only = True` keeps it off the command line (see `args.string`).
    fn string_list<'v>(
        #[starlark(require = named, default = false)] required: bool,
        #[starlark(require = named, default = NoneOr::None)] default: NoneOr<UnpackList<String>>,
        #[starlark(require = named, default = NoneOr::None)] short: NoneOr<String>,
        #[starlark(require = named, default = NoneOr::None)] long: NoneOr<String>,
        #[starlark(require = named, default = NoneOr::None)] description: NoneOr<String>,
        #[starlark(require = named, default = false)] config_only: bool,
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
        let config_only = validated_config_only(config_only, required, &short, &long)?;
        Ok(Arg::StringList {
            required,
            default: default.into_option().map(|it| it.items).unwrap_or_default(),
            short: short.into_option(),
            long: validated_long(long)?,
            description: description.into_option(),
            config_only,
        })
    }

    /// Defines a boolean flag. Use `--flag_name` (true) or `--flag_name=false`.
    ///
    /// Use `long = "override-name"` to override the default kebab-case derivation.
    /// `config_only = True` keeps it off the command line (see `args.string`).
    fn boolean<'v>(
        #[starlark(require = named, default = false)] required: bool,
        #[starlark(require = named)] default: Option<bool>,
        #[starlark(require = named, default = NoneOr::None)] short: NoneOr<String>,
        #[starlark(require = named, default = NoneOr::None)] long: NoneOr<String>,
        #[starlark(require = named, default = NoneOr::None)] description: NoneOr<String>,
        _eval: &mut Evaluator<'v, '_, '_>,
        #[starlark(require = named, default = false)] config_only: bool,
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
        let config_only = validated_config_only(config_only, required, &short, &long)?;
        Ok(Arg::Boolean {
            required,
            default: default.unwrap_or_default(),
            short: short.into_option(),
            long: validated_long(long)?,
            description: description.into_option(),
            config_only,
        })
    }

    /// Defines a boolean list flag that can be specified multiple times.
    ///
    /// Use `long = "override-name"` to override the default kebab-case derivation.
    /// `config_only = True` keeps it off the command line (see `args.string`).
    fn boolean_list<'v>(
        #[starlark(require = named, default = false)] required: bool,
        #[starlark(require = named, default = NoneOr::None)] default: NoneOr<UnpackList<bool>>,
        #[starlark(require = named, default = NoneOr::None)] short: NoneOr<String>,
        #[starlark(require = named, default = NoneOr::None)] long: NoneOr<String>,
        #[starlark(require = named, default = NoneOr::None)] description: NoneOr<String>,
        #[starlark(require = named, default = false)] config_only: bool,
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
        let config_only = validated_config_only(config_only, required, &short, &long)?;
        Ok(Arg::BooleanList {
            required,
            default: default.into_option().map(|it| it.items).unwrap_or_default(),
            short: short.into_option(),
            long: validated_long(long)?,
            description: description.into_option(),
            config_only,
        })
    }

    /// Defines an integer flag.
    ///
    /// Use `long = "override-name"` to override the default kebab-case derivation.
    /// `config_only = True` keeps it off the command line (see `args.string`).
    fn int<'v>(
        #[starlark(require = named, default = false)] required: bool,
        #[starlark(require = named)] default: Option<i32>,
        #[starlark(require = named, default = NoneOr::None)] short: NoneOr<String>,
        #[starlark(require = named, default = NoneOr::None)] long: NoneOr<String>,
        #[starlark(require = named, default = NoneOr::None)] description: NoneOr<String>,
        #[starlark(require = named, default = false)] config_only: bool,
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
        let config_only = validated_config_only(config_only, required, &short, &long)?;
        Ok(Arg::Int {
            required,
            default: default.unwrap_or_default(),
            short: short.into_option(),
            long: validated_long(long)?,
            description: description.into_option(),
            config_only,
        })
    }

    /// Defines an integer list flag that can be specified multiple times.
    ///
    /// Use `long = "override-name"` to override the default kebab-case derivation.
    /// `config_only = True` keeps it off the command line (see `args.string`).
    fn int_list<'v>(
        #[starlark(require = named, default = false)] required: bool,
        #[starlark(require = named, default = NoneOr::None)] default: NoneOr<UnpackList<i32>>,
        #[starlark(require = named, default = NoneOr::None)] short: NoneOr<String>,
        #[starlark(require = named, default = NoneOr::None)] long: NoneOr<String>,
        #[starlark(require = named, default = NoneOr::None)] description: NoneOr<String>,
        #[starlark(require = named, default = false)] config_only: bool,
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
        let config_only = validated_config_only(config_only, required, &short, &long)?;
        Ok(Arg::IntList {
            required,
            default: default.into_option().map(|it| it.items).unwrap_or_default(),
            short: short.into_option(),
            long: validated_long(long)?,
            description: description.into_option(),
            config_only,
        })
    }

    /// Defines an unsigned integer flag.
    ///
    /// Use `long = "override-name"` to override the default kebab-case derivation.
    /// `config_only = True` keeps it off the command line (see `args.string`).
    fn uint<'v>(
        #[starlark(require = named, default = false)] required: bool,
        #[starlark(require = named)] default: Option<u32>,
        #[starlark(require = named, default = NoneOr::None)] short: NoneOr<String>,
        #[starlark(require = named, default = NoneOr::None)] long: NoneOr<String>,
        #[starlark(require = named, default = NoneOr::None)] description: NoneOr<String>,
        #[starlark(require = named, default = false)] config_only: bool,
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
        let config_only = validated_config_only(config_only, required, &short, &long)?;
        Ok(Arg::UInt {
            required,
            default: default.unwrap_or_default(),
            short: short.into_option(),
            long: validated_long(long)?,
            description: description.into_option(),
            config_only,
        })
    }

    /// Defines an unsigned integer list flag that can be specified multiple times.
    ///
    /// Use `long = "override-name"` to override the default kebab-case derivation.
    /// `config_only = True` keeps it off the command line (see `args.string`).
    fn uint_list<'v>(
        #[starlark(require = named, default = false)] required: bool,
        #[starlark(require = named, default = NoneOr::None)] default: NoneOr<UnpackList<u32>>,
        #[starlark(require = named, default = NoneOr::None)] short: NoneOr<String>,
        #[starlark(require = named, default = NoneOr::None)] long: NoneOr<String>,
        #[starlark(require = named, default = NoneOr::None)] description: NoneOr<String>,
        #[starlark(require = named, default = false)] config_only: bool,
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
        let config_only = validated_config_only(config_only, required, &short, &long)?;
        Ok(Arg::UIntList {
            required,
            default: default.into_option().map(|it| it.items).unwrap_or_default(),
            short: short.into_option(),
            long: validated_long(long)?,
            description: description.into_option(),
            config_only,
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
        // storing. For a list of strings, `args.string_list(config_only = True)` is
        // config-only in the same way and does carry its default.
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
