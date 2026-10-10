use std::fs::File;
use std::io::{BufWriter, Write};
use std::thread::{self, JoinHandle};

use allocative::Allocative;
use axl_proto::tools::protos::ExecLogEntry;
use derive_more::Display;
use fibre::RecvError;
use fibre::spmc::Receiver;
use prost::Message;
use starlark::starlark_simple_value;
use starlark::values;
use starlark::values::starlark_value;
use starlark::values::{NoSerialize, ProvidesStaticType, UnpackValue, ValueLike};

use super::retry::{SinkError, SinkOutcome};

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

impl ExecLogSink {
    /// Spawns a thread that reads decoded `ExecLogEntry` values from `recv` and
    /// writes them to `path` in varint-length-prefixed binary proto format.
    ///
    /// I/O errors surface as `Err(SinkError)`; the caller decides whether to
    /// fail the task (e.g. by inspecting the sink's outcome).
    pub fn spawn_file(recv: Receiver<ExecLogEntry>, path: String) -> JoinHandle<SinkOutcome> {
        thread::spawn(move || {
            let err = |last_error: String| SinkError { last_error };
            let file = File::create(&path)
                .map_err(|e| err(format!("ExecLog: failed to create '{path}': {e}")))?;
            let mut file = BufWriter::new(file);
            loop {
                match recv.recv() {
                    Ok(entry) => {
                        file.write_all(&entry.encode_length_delimited_to_vec())
                            .map_err(|e| err(format!("ExecLog: write to '{path}' failed: {e}")))?;
                    }
                    Err(RecvError::Disconnected) => break,
                }
            }
            Ok(())
        })
    }
}
