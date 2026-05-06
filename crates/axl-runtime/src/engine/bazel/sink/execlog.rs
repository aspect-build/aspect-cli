use std::fs::File;
use std::io::{BufWriter, Write};
use std::thread::{self, JoinHandle};

use allocative::Allocative;
use axl_proto::tools::protos::ExecLogEntry;
use derive_more::Display;
use fibre::RecvError;
use fibre::spmc::Receiver;
use prost::Message;
use starlark::StarlarkResultExt;
use starlark::starlark_simple_value;
use starlark::values;
use starlark::values::starlark_value;
use starlark::values::{NoSerialize, ProvidesStaticType, UnpackValue, ValueLike};

use super::retry::{ErrorStrategy, SinkError, SinkOutcome};

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

    fn unpack_value_impl(value: values::Value<'v>) -> Result<Option<Self>, Self::Error> {
        let value = value
            .downcast_ref_err::<ExecLogSink>()
            .into_anyhow_result()?;
        Ok(Some(value.clone()))
    }
}

impl ExecLogSink {
    /// Spawns a thread that reads decoded `ExecLogEntry` values from `recv` and
    /// writes them to `path` in varint-length-prefixed binary proto format.
    ///
    /// I/O errors surface as `Err(SinkError { strategy: Abort, .. })` so the
    /// build fails cleanly instead of leaving a half-written log behind.
    pub fn spawn_file(recv: Receiver<ExecLogEntry>, path: String) -> JoinHandle<SinkOutcome> {
        thread::spawn(move || {
            let abort = |last_error: String| SinkError {
                strategy: ErrorStrategy::Abort,
                last_error,
            };
            let file = File::create(&path)
                .map_err(|e| abort(format!("ExecLog: failed to create '{path}': {e}")))?;
            let mut file = BufWriter::new(file);
            loop {
                match recv.recv() {
                    Ok(entry) => {
                        file.write_all(&entry.encode_length_delimited_to_vec())
                            .map_err(|e| {
                                abort(format!("ExecLog: write to '{path}' failed: {e}"))
                            })?;
                    }
                    Err(RecvError::Disconnected) => break,
                }
            }
            Ok(())
        })
    }
}
