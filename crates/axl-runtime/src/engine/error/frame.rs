//! The frames of an error's `stacktrace`.

use std::fmt::{self, Display};

use allocative::Allocative;
use starlark::environment::{Methods, MethodsBuilder, MethodsStatic};
use starlark::starlark_module;
use starlark::starlark_simple_value;
use starlark::values::none::NoneOr;
use starlark::values::{
    NoSerialize, ProvidesStaticType, StarlarkValue, Value, ValueLike, starlark_value,
};

/// One frame of an error's `stacktrace`.
#[derive(Debug, ProvidesStaticType, NoSerialize, Allocative)]
pub struct ErrorFrame {
    name: String,
    /// File, 1-based line and 1-based column of the call, when known.
    location: Option<(String, u32, u32)>,
}

impl ErrorFrame {
    pub(super) fn new(name: &str, location: Option<&starlark::codemap::FileSpan>) -> Self {
        let location = location.map(|span| {
            let begin = span.resolve_span().begin;
            (
                span.filename().to_owned(),
                begin.line as u32 + 1,
                begin.column as u32 + 1,
            )
        });
        Self {
            name: name.to_owned(),
            location,
        }
    }
}

impl Display for ErrorFrame {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match &self.location {
            Some((path, line, column)) => write!(f, "{}:{line}:{column}: in {}", path, self.name),
            None => write!(f, "<native>: in {}", self.name),
        }
    }
}

starlark_simple_value!(ErrorFrame);

#[starlark_value(type = "error_frame")]
impl<'v> StarlarkValue<'v> for ErrorFrame {
    fn get_methods() -> Option<&'static Methods> {
        static RES: MethodsStatic = MethodsStatic::new("error_frame_methods", error_frame_methods);
        Some(RES.methods())
    }
}

fn this_frame<'v>(this: Value<'v>) -> &'v ErrorFrame {
    this.downcast_ref::<ErrorFrame>()
        .expect("frame attribute bound on a non-frame value")
}

#[starlark_module]
fn error_frame_methods(builder: &mut MethodsBuilder) {
    /// The function running in this frame.
    #[starlark(attribute)]
    fn name<'v>(this: Value<'v>) -> starlark::Result<String> {
        Ok(this_frame(this).name.clone())
    }

    /// The file of the call, or `None` for a native frame.
    #[starlark(attribute)]
    fn path<'v>(this: Value<'v>) -> starlark::Result<NoneOr<String>> {
        Ok(NoneOr::from_option(
            this_frame(this).location.as_ref().map(|l| l.0.clone()),
        ))
    }

    /// The 1-based line of the call, or `None` for a native frame.
    #[starlark(attribute)]
    fn line<'v>(this: Value<'v>) -> starlark::Result<NoneOr<u32>> {
        Ok(NoneOr::from_option(
            this_frame(this).location.as_ref().map(|l| l.1),
        ))
    }

    /// The 1-based column of the call, or `None` for a native frame.
    #[starlark(attribute)]
    fn column<'v>(this: Value<'v>) -> starlark::Result<NoneOr<u32>> {
        Ok(NoneOr::from_option(
            this_frame(this).location.as_ref().map(|l| l.2),
        ))
    }
}
