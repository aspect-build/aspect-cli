use std::fs::File;
use std::io::{BufWriter, Write};
use std::thread::{self, JoinHandle};

use axl_proto::tools::protos::ExecLogEntry;
use fibre::RecvError;
use fibre::spmc::Receiver;
use prost::Message;

use crate::fanout::{SinkError, SinkOutcome};

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
