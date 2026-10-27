use allocative::Allocative;
use derive_more::Display;
use starlark::starlark_simple_value;
use starlark::values;
use starlark::values::starlark_value;
use starlark::values::{NoSerialize, ProvidesStaticType, UnpackValue, ValueLike};

/// Sink types for execution log output.
///
/// | Variant | Format |
/// |---|---|
/// | `File` | Varint-length-prefixed binary proto, no zstd (decoded entries re-encoded) |
/// | `CompactFile` | Raw zstd-compressed bytes (identical to `--execution_log_compact_file`) |
#[derive(Debug, Display, ProvidesStaticType, NoSerialize, Allocative, Clone)]
#[display("<bazel.execlog.ExecLogSink>")]
pub enum ExecLogSink {
    File { path: String },
    CompactFile { path: String },
}

starlark_simple_value!(ExecLogSink);

#[starlark_value(type = "bazel.execlog.ExecLogSink")]
impl<'v> values::StarlarkValue<'v> for ExecLogSink {}

impl<'v> UnpackValue<'v> for ExecLogSink {
    type Error = anyhow::Error;

    // `Ok(None)` (not `Err`) on type mismatch so Either's UnpackValue can fall
    // through to the next branch — `execution_log=` takes a list of
    // `ExecLogSink | ExecLogIter`, and an `Err` here would reject every iterator
    // handle before the iterator branch was ever tried. Same reasoning, and the
    // same shape, as `BuildEventSink`.
    fn unpack_value_impl(value: values::Value<'v>) -> Result<Option<Self>, Self::Error> {
        Ok(value.downcast_ref::<ExecLogSink>().cloned())
    }
}
