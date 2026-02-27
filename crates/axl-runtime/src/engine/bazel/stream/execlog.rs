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

use super::util::MultiTeeReader;
use super::util::read_varint;
use thiserror::Error;
use zstd::Decoder;

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

/// Wraps a `Read` source, blocking on empty reads until real data arrives.
///
/// Some `Read` implementations (e.g. [`galvanize::StreamingFile`]) return `Ok(0)` to signal
/// "no data yet, try again" while the writer is still active. Framing layers like the zstd
/// `Decoder` interpret `Ok(0)` as EOF and error with "incomplete frame". This adapter sits
/// between such a source and the decoder, converting empty reads into a brief sleep-and-retry
/// so the decoder always receives either real bytes or a terminal error.
struct RetryRead<R: Read> {
    inner: R,
}

impl<R: Read> Read for RetryRead<R> {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        loop {
            match self.inner.read(buf) {
                Ok(0) => std::thread::sleep(std::time::Duration::from_millis(1)),
                other => return other,
            }
        }
    }
}

#[derive(Debug)]
pub struct ExecLogStream {
    handle: JoinHandle<Result<(), ExecLogStreamError>>,
    // Holds the initial subscriber clone. Kept as Option so join() can drop it
    // before the thread finishes. In fibre's SPMC broadcast ring buffer every
    // Receiver clone is an independent subscriber whose tail the sender must not
    // lap; an unconsumed clone prevents the Closed signal that tells the producer
    // to stop decoding. Dropping it first means the sender sees Closed on the
    // first try_send when no external subscribers exist, skipping all decoding.
    recv: Option<Receiver<ExecLogEntry>>,
}

impl ExecLogStream {
    /// Spawn the execlog reader thread using a FIFO (named pipe).
    ///
    /// # Warning — do not use when `--build_event_binary_file` is also a FIFO
    ///
    /// Bazel checksums the compact execlog file after writing it in order to populate
    /// the `build_tool_logs` BEP event. A FIFO cannot be re-read for this purpose, so
    /// Bazel stalls mid-build trying to seek back, which in turn prevents the BEP FIFO
    /// from being flushed, causing a deadlock. See:
    /// <https://github.com/bazelbuild/bazel/issues/28800>
    ///
    /// Use [`spawn_with_file`](Self::spawn_with_file) instead. This method is retained
    /// for contexts where the BEP stream is not active and the checksum path is not hit.
    #[allow(dead_code)]
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
                        Ok(()) | Err(TrySendError::Sent(_)) => {}
                        // Channel full: iterator consumer is slow, drop entry.
                        Err(TrySendError::Full(_)) => {}
                        // No receivers left: skip decoding for remaining entries.
                        Err(TrySendError::Closed(_)) => has_readers = false,
                    }
                }

                Ok(())
            };

            loop {
                match read() {
                    Ok(()) => continue,
                    // End of stream.
                    Err(ExecLogStreamError::IO(err)) if err.kind() == io::ErrorKind::BrokenPipe => {
                        sender.close()?;
                        out_raw.get_mut().get_mut().flush()?;
                        return Ok(());
                    }
                    Err(err) => return Err(err),
                }
            }
        });
        Ok(Self {
            handle,
            recv: Some(recv),
        })
    }

    /// Spawn the execlog reader thread for a regular file.
    ///
    /// `pid` is the Bazel server process ID, used to detect when Bazel has finished
    /// writing the file. `out_path` is the file Bazel will write
    /// `--execution_log_compact_file` to. Pass `Some(path)` to reuse an existing sink
    /// path (e.g. a `CompactFile` sink so Bazel writes directly to the caller's
    /// destination without a tee step). Pass `None` to have a UUID-named temp file
    /// created automatically.
    ///
    /// The thread streams the file as Bazel writes it using [`galvanize::StreamingFile`],
    /// which busy-polls for file existence at open time and retries reads while Bazel
    /// holds the file open. It self-terminates when Bazel closes the file.
    pub fn spawn_with_file(
        pid: u32,
        out_path: Option<PathBuf>,
        compact_sink_paths: Vec<String>,
        has_file_sinks: bool,
    ) -> io::Result<(PathBuf, Self)> {
        let out = out_path.unwrap_or_else(|| {
            env::temp_dir().join(format!("execlog-out-{}.bin", uuid::Uuid::new_v4()))
        });
        let (mut sender, recv) = bounded::<ExecLogEntry>(1000);
        let path = out.clone();
        let handle = thread::spawn(move || {
            let mut buf: Vec<u8> = Vec::with_capacity(1024 * 5);
            // 10 is the maximum size of a varint so start with that size.
            buf.resize(10, 0);

            let out_raw = galvanize::StreamingFile::open(path.clone(), pid)?;
            let writers = compact_sink_paths
                .iter()
                .map(|p| Ok(BufWriter::new(File::create(p)?)))
                .collect::<io::Result<Vec<_>>>()?;
            let out_raw = MultiTeeReader {
                inner: out_raw,
                writers,
            };
            // RetryRead prevents zstd from seeing Ok(0) ("no data yet") as EOF.
            let out_raw = RetryRead { inner: out_raw };
            let mut out_raw = Decoder::new(out_raw)?;

            // Only used in the try_send path (no file sinks).
            let mut has_readers = true;

            let mut read = || -> Result<(), ExecLogStreamError> {
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
                        Ok(()) | Err(TrySendError::Sent(_)) => {}
                        // Channel full: iterator consumer is slow, drop entry.
                        Err(TrySendError::Full(_)) => {}
                        // No receivers left: skip decoding for remaining entries.
                        Err(TrySendError::Closed(_)) => has_readers = false,
                    }
                }

                Ok(())
            };

            loop {
                match read() {
                    Ok(()) => continue,
                    // BrokenPipe signals that Bazel closed the file (end of stream).
                    Err(ExecLogStreamError::IO(err)) if err.kind() == io::ErrorKind::BrokenPipe => {
                        sender.close()?;
                        out_raw.get_mut().get_mut().inner.flush()?;
                        return Ok(());
                    }
                    Err(err) => return Err(err),
                }
            }
        });

        Ok((
            out,
            Self {
                handle,
                recv: Some(recv),
            },
        ))
    }

    pub fn receiver(&self) -> Receiver<ExecLogEntry> {
        self.recv
            .as_ref()
            .expect("receiver() called after join()")
            .clone()
    }

    /// Wait for the execlog stream to finish.
    ///
    /// Drops the struct's `recv` clone so that if no external subscriber exists
    /// the first `try_send` returns `Closed` and remaining bytes are drained
    /// without proto decoding. Then waits for the thread to exit.
    pub fn join(mut self) -> Result<(), ExecLogStreamError> {
        self.recv.take();
        self.handle.join().expect("join error")
    }
}
