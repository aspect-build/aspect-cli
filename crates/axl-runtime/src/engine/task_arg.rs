use std::convert::Infallible;
use std::fmt::Display;
use std::u32;

use allocative::Allocative;
use starlark::environment::GlobalsBuilder;
use starlark::eval::Evaluator;
use starlark::starlark_module;
use starlark::starlark_simple_value;
use starlark::values::list::UnpackList;
use starlark::values::none::NoneOr;
use starlark::values::starlark_value;
use starlark::values::starlark_value_as_type::StarlarkValueAsType;
use starlark::values::NoSerialize;
use starlark::values::ProvidesStaticType;
use starlark::values::StarlarkValue;
use starlark::values::UnpackValue;
use starlark::values::Value;
use starlark::values::ValueLike;
use starlark::ErrorKind;

#[derive(Clone, Debug, ProvidesStaticType, NoSerialize, Allocative)]
pub enum TaskArg {
    String {
        required: bool,
        default: String,
    },
    Boolean {
        required: bool,
        default: bool,
    },
    Int {
        required: bool,
        default: i32,
    },
    UInt {
        required: bool,
        default: u32,
    },
    Positional {
        minimum: u32,
        maximum: u32,
        default: Option<Vec<String>>,
    },
    TrailingVarArgs,
}

/// Documentation here
#[starlark_value(type = "args.TaskArg")]
impl<'v> StarlarkValue<'v> for TaskArg {}

starlark_simple_value!(TaskArg);

impl Display for TaskArg {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::String { .. } => write!(f, "<args.TaskArg: string>"),
            Self::Boolean { .. } => write!(f, "<args.TaskArg: boolean>"),
            Self::Int { .. } => write!(f, "<args.TaskArg: int>"),
            Self::UInt { .. } => write!(f, "<args.TaskArg: uint>"),
            Self::Positional { .. } => write!(f, "<args.TaskArg: positional>"),
            Self::TrailingVarArgs => write!(f, "<args.TaskArg: trailing variable arguments>"),
        }
    }
}

impl<'v> UnpackValue<'v> for TaskArg {
    type Error = Infallible;

    fn unpack_value_impl(value: Value<'v>) -> Result<Option<Self>, Self::Error> {
        Ok(value.downcast_ref::<Self>().map(|value| value.clone()))
    }
}

#[starlark_module]
pub fn register_globals(globals: &mut GlobalsBuilder) {
    const Args: StarlarkValueAsType<TaskArg> = StarlarkValueAsType::new();

    /// Defines a positional argument that accepts a range of values, with a required minimum
    /// number of values and an optional maximum number of values.
    ///
    ///
    /// # Examples
    /// ```python
    /// # Take one positional argument with no dashes.
    /// task(
    ///  args = { "named": args.positional() }
    /// )
    /// ```
    ///
    /// ```python
    /// # Take two positional argument with no dashes.
    /// task(
    ///  args = { "named": args.positional(minimum = 2, maximum = 2) }
    /// )
    /// ```
    fn positional<'v>(
        #[starlark(require = named, default = 0)] minimum: u32,
        #[starlark(require = named, default = 1)] maximum: u32,
        #[starlark(require = named, default = NoneOr::None)] default: NoneOr<UnpackList<String>>,
    ) -> starlark::Result<TaskArg> {
        Ok(TaskArg::Positional {
            minimum: minimum,
            maximum: maximum,
            default: default.into_option().map(|it| it.items),
        })
    }

    /// Defines a trailing variable argument that captures the remaining arguments without further parsing.
    /// Only one such argument is permitted, and it must be the last in the sequence.
    ///
    /// # Examples
    /// ```python
    /// task(
    ///   args = {
    ///     # take one positional argument with no dashes.
    ///     "target": args.positional(minimum = 0, maximum = 1),
    ///     # take rest of the commandline
    ///     "run_args": args.trailing_var_args()
    ///   }
    /// )
    /// ```
    fn trailing_var_args<'v>() -> starlark::Result<TaskArg> {
        Ok(TaskArg::TrailingVarArgs {})
    }

    /// Defines a string flag that can be specified as `--flag_name=flag_value`.
    ///
    /// # Examples
    /// ```python
    /// task(
    ///   args = {
    ///     "bes_backend": args.string(),
    ///   }
    /// )
    /// ```
    fn string<'v>(
        #[starlark(require = named, default = false)] required: bool,
        #[starlark(require = named)] default: Option<String>,
    ) -> starlark::Result<TaskArg> {
        if required && default.is_some() {
            return Err(starlark::Error::new_kind(ErrorKind::Function(
                anyhow::anyhow!("`required` and `default` are both set."),
            )));
        }
        Ok(TaskArg::String {
            required,
            default: default.unwrap_or_default(),
        })
    }

    /// Defines a boolean flag that can be specified as `--flag_name=true|false`
    /// or simply `--flag_name`, which is equivalent to `--flag_name=true`.
    ///
    /// # Examples
    /// ```python
    /// task(
    ///   args = {
    ///     "color": args.boolean(),
    ///   }
    /// )
    /// ```
    fn boolean<'v>(
        #[starlark(require = named, default = false)] required: bool,
        #[starlark(require = named )] default: Option<bool>,
        _eval: &mut Evaluator<'v, '_, '_>,
    ) -> starlark::Result<TaskArg> {
        if required && default.is_some() {
            return Err(starlark::Error::new_kind(ErrorKind::Function(
                anyhow::anyhow!("`required` and `default` are both set."),
            )));
        }
        Ok(TaskArg::Boolean {
            required,
            default: default.unwrap_or_default(),
        })
    }

    /// Creates an integer flag that can be set as `--flag_name=flag_value`
    /// or `--flag_name=flag_value`.
    ///
    /// # Examples
    /// ```python
    /// task(
    ///   args = {
    ///     "color": args.int(),
    ///   }
    /// )
    /// ```
    fn int<'v>(
        #[starlark(require = named, default = false)] required: bool,
        #[starlark(require = named)] default: Option<i32>,
    ) -> starlark::Result<TaskArg> {
        if required && default.is_some() {
            return Err(starlark::Error::new_kind(ErrorKind::Function(
                anyhow::anyhow!("`required` and `default` are both set."),
            )));
        }
        Ok(TaskArg::Int {
            required,
            default: default.unwrap_or_default(),
        })
    }

    /// Defines an unsigned integer flag that can be specified using the format `--flag_name=flag_value`.
    ///
    /// # Examples
    /// ```python
    /// task(
    ///   args = {
    ///     "color": args.uint(),
    ///   }
    /// )
    /// ```
    fn uint<'v>(
        #[starlark(require = named, default = false)] required: bool,
        #[starlark(require = named)] default: Option<u32>,
    ) -> starlark::Result<TaskArg> {
        if required && default.is_some() {
            return Err(starlark::Error::new_kind(ErrorKind::Function(
                anyhow::anyhow!("`required` and `default` are both set."),
            )));
        }
        Ok(TaskArg::UInt {
            required,
            default: default.unwrap_or_default(),
        })
    }
}
