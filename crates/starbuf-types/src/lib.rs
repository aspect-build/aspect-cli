use prost_types::{Duration, Timestamp};

#[derive(
    ::starlark::values::ProvidesStaticType,
    ::derive_more::Display,
    ::starlark::values::Trace,
    ::starlark::values::NoSerialize,
    ::allocative::Allocative,
    Debug,
)]
#[display("Duration")]
pub struct SBDuration(#[allocative(skip)] pub Duration);

#[starlark::values::starlark_value(type = "duration")]
impl<'v> starlark::values::StarlarkValue<'v> for SBDuration {}

#[derive(
    ::starlark::values::ProvidesStaticType,
    ::derive_more::Display,
    ::starlark::values::Trace,
    ::starlark::values::NoSerialize,
    ::allocative::Allocative,
    Debug,
)]
#[display("Timestamp")]
pub struct SBTimestamp(#[allocative(skip)] pub Timestamp);

#[starlark::values::starlark_value(type = "Timestamp")]
impl<'v> starlark::values::StarlarkValue<'v> for SBTimestamp {}
