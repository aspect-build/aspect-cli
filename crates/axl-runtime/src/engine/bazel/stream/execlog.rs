use axl_proto::tools::protos::ExecLogEntry;
use fibre::spmc::{Receiver, bounded};
use fibre::{CloseError, SendError, TrySendError};
use prost::Message;
use std::fmt::Debug;
use std::fs::File;
use std::io;
use std::io::{BufWriter, Read, Write};
use std::path::PathBuf;
use std::thread::JoinHandle;
use std::{env, thread};
use thiserror::Error;
use zstd::Decoder;

use super::util::read_varint;

#[derive(Error, Debug)]
pub enum ExecLogStreamError {
    #[error("io error: {0}")]
    IO(#[from] std::io::Error),
    #[error("prost decode error: {0}")]
    ProstDecode(#[from] prost::DecodeError),
    #[error("send error: {0}")]
    Send(#[from] SendError),
    #[error("close error: {0}")]
    Close(#[from] CloseError),
}

/// Wraps a `Read` source and tees every byte read to one or more `BufWriter<File>` sinks.
///
/// Used to intercept the raw zstd-compressed bytes coming from Bazel's named pipe
/// *before* decompression, allowing `CompactFile` sinks to capture Bazel-compatible
/// `--execution_log_compact_file` output without a second compression step.
struct MultiTeeReader<R: Read> {
    inner: R,
    writers: Vec<BufWriter<File>>,
}

impl<R: Read> Read for MultiTeeReader<R> {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        let n = self.inner.read(buf)?;
        for w in &mut self.writers {
            w.write_all(&buf[..n])?;
        }
        Ok(n)
    }
}

#[derive(Debug)]
pub struct ExecLogStream {
    handle: JoinHandle<Result<(), ExecLogStreamError>>,
    recv: Receiver<ExecLogEntry>,
}

impl ExecLogStream {
    pub fn spawn_with_pipe(
        pid: u32,
        compact_sink_paths: Vec<String>,
        has_file_sinks: bool,
    ) -> io::Result<(PathBuf, Self)> {
        let out = env::temp_dir().join(format!("execlog-out-{}.bin", uuid::Uuid::new_v4()));
        let stream = Self::spawn(out.clone(), pid, compact_sink_paths, has_file_sinks)?;
        Ok((out, stream))
    }

    /// Spawn the execlog reader thread.
    ///
    /// ## Send strategy
    ///
    /// `has_file_sinks` controls how decoded entries are sent to the channel:
    ///
    /// - `true` — blocking [`Sender::send`]. File-sink threads must receive every entry
    ///   to produce a complete output file, so the producer waits for the channel to drain
    ///   rather than dropping entries. The build may slow under sustained I/O pressure, but
    ///   it will not deadlock because the sink threads are always consuming.
    ///
    /// - `false` — non-blocking [`Sender::try_send`]. Used when the only consumer is the
    ///   optional `execution_logs()` iterator. A full channel means the caller is not
    ///   consuming fast enough; entries are dropped rather than stalling the build.
    ///   Once all receiver clones are gone (`Closed`), decoding is skipped entirely.
    ///
    /// `CompactFile` sinks are unaffected by this flag — raw bytes are always tee'd
    /// by `MultiTeeReader` before decoding.
    pub fn spawn(
        path: PathBuf,
        pid: u32,
        compact_sink_paths: Vec<String>,
        has_file_sinks: bool,
    ) -> io::Result<Self> {
        let (mut sender, recv) = bounded::<ExecLogEntry>(1000);
        let handle = thread::spawn(move || {
            let mut buf: Vec<u8> = Vec::with_capacity(1024 * 5);
            // 10 is the maximum size of a varint so start with that size.
            buf.resize(10, 0);
            let out_raw =
                galvanize::Pipe::new(path.clone(), galvanize::RetryPolicy::IfOpenForPid(pid))?;
            let writers = compact_sink_paths
                .iter()
                .map(|p| Ok(BufWriter::new(File::create(p)?)))
                .collect::<io::Result<Vec<_>>>()?;
            let out_raw = MultiTeeReader {
                inner: out_raw,
                writers,
            };
            let mut out_raw = Decoder::new(out_raw)?;

            // Only used in the try_send path (no file sinks).
            // Set to false when try_send returns Closed, skipping future decodes.
            let mut has_readers = true;

            let mut read = || -> Result<(), ExecLogStreamError> {
                // varint size can be somewhere between 1 to 10 bytes.
                let size = read_varint(&mut out_raw)?;
                if size > buf.len() {
                    buf.resize(size, 0);
                }

                out_raw.read_exact(&mut buf[0..size])?;

                if has_file_sinks {
                    let entry = ExecLogEntry::decode(&buf[0..size])?;
                    sender.send(entry)?;
                } else if has_readers {
                    let entry = ExecLogEntry::decode(&buf[0..size])?;
                    match sender.try_send(entry) {
                        Ok(()) => {}
                        // Channel full: iterator consumer is slow, drop entry.
                        Err(TrySendError::Full(_)) => {}
                        // No receivers left: skip decoding for remaining entries.
                        Err(TrySendError::Closed(_)) => {
                            has_readers = false;
                        }
                        Err(TrySendError::Sent(_)) => {}
                    }
                }

                Ok(())
            };

            loop {
                let result = read();

                // event decoding was succesfull move to the next.
                if result.is_ok() {
                    continue;
                }

                match result.unwrap_err() {
                    // this marks the end of the stream
                    ExecLogStreamError::IO(err) if err.kind() == io::ErrorKind::BrokenPipe => {
                        sender.close()?;
                        return Ok(());
                    }
                    err => return Err(err),
                }
            }
        });
        Ok(Self { handle, recv })
    }

    pub fn receiver(&self) -> Receiver<ExecLogEntry> {
        self.recv.clone()
    }

    pub fn join(self) -> Result<(), ExecLogStreamError> {
        self.handle.join().expect("join error")
    }
}
