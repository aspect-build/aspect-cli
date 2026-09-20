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

use super::super::iter::execlog::DecodeFailure;
use super::super::sink::retry::SinkOutcome;
use super::util::{MultiTeeReader, read_varint};
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

/// Leading bytes of every zstd frame, little-endian 0xFD2FB528.
const ZSTD_MAGIC: [u8; 4] = [0x28, 0xB5, 0x2F, 0xFD];

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
    /// Decoded-file sink writer threads owned by this stream — joined in `join()`.
    file_sink_handles: Vec<JoinHandle<SinkOutcome>>,
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
                let (size, _) = read_varint(&mut out_raw)?;
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
            file_sink_handles: vec![],
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
                let (size, _) = read_varint(&mut out_raw)?;
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
                file_sink_handles: vec![],
            },
        ))
    }

    /// Decode a compact execution log that already exists on disk.
    ///
    /// Unlike [`spawn`](Self::spawn) and [`spawn_with_file`](Self::spawn_with_file), which
    /// follow a file Bazel is still writing, this reads a finished
    /// `--execution_log_compact_file` artifact: a plain zstd frame of
    /// varint-length-prefixed `ExecLogEntry` messages. The end of the file is the end of
    /// the stream, so EOF terminates rather than `BrokenPipe`.
    ///
    /// Sends block. A file this is pointed at was produced by a build that already
    /// finished, so there is no build to slow down, and silently dropping entries the way
    /// the live path does would make a diff wrong rather than slow.
    ///
    /// ## Truncation is usually, not always, detected
    ///
    /// A log cut off part-way through sets `failure` and ends the stream early, so a
    /// caller that checks it does not compare a prefix and call it a clean result. The
    /// detection comes from zstd, which reports `incomplete frame` when it runs out of
    /// input mid-block. When the cut lands exactly on a block boundary, the decoder sees
    /// a clean end of input and the entries before it are a structurally valid stream, so
    /// the read looks successful. Measured against a real 134 KB log, cuts at 40% and
    /// beyond were reported; cuts at 10% and 25% were not.
    ///
    /// Closing that last gap means checking zstd's end-of-frame marker, which the `zstd`
    /// crate's `Decoder` does not expose; it needs `zstd_safe::DCtx` directly. Worth doing
    /// if a partial log is ever mistaken for a real answer in practice.
    pub fn spawn_from_path(path: PathBuf, failure: DecodeFailure) -> io::Result<Self> {
        // Fail here, on the caller's thread, for the mistakes worth a traceback: a
        // path that does not exist, or a file that is not a zstd frame at all.
        // `Decoder::new` reads lazily, so it accepts a text file and only complains
        // once the consumer starts iterating; checking the magic number is what makes
        // "you pointed this at your BEP file" an error rather than an empty result.
        // A log that is truncated or corrupt part-way through still surfaces through
        // `failure`, because that cannot be known without reading it all.
        let mut magic = [0u8; 4];
        File::open(&path)?.read_exact(&mut magic).map_err(|e| {
            io::Error::new(
                e.kind(),
                format!("{path:?} is too short to be a zstd frame"),
            )
        })?;
        if magic != ZSTD_MAGIC {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                format!(
                    "{path:?} is not a zstd frame, so it is not a compact execution log. \
                     Pass the file written by --execution_log_compact_file."
                ),
            ));
        }

        let (mut sender, recv) = bounded::<ExecLogEntry>(1000);
        let report = failure.clone();
        let handle = thread::spawn(move || {
            let mut buf: Vec<u8> = Vec::with_capacity(1024 * 5);
            // 10 is the maximum size of a varint so start with that size.
            buf.resize(10, 0);

            let mut out_raw = Decoder::new(File::open(&path)?)?;

            let mut read = || -> Result<bool, ExecLogStreamError> {
                let (size, _) = match read_varint(&mut out_raw) {
                    Ok(it) => it,
                    // A clean end of file between entries: the log is fully read.
                    Err(err) if err.kind() == io::ErrorKind::UnexpectedEof => return Ok(false),
                    Err(err) => return Err(err.into()),
                };
                if size > buf.len() {
                    buf.resize(size, 0);
                }
                out_raw.read_exact(&mut buf[0..size])?;
                sender.send(ExecLogEntry::decode(&buf[0..size])?)?;
                Ok(true)
            };

            loop {
                match read() {
                    Ok(true) => continue,
                    Ok(false) => {
                        sender.close()?;
                        return Ok(());
                    }
                    Err(err) => {
                        // Record before closing: a consumer blocked in `recv` wakes on
                        // the close and may read `error()` immediately afterwards.
                        if let Ok(mut slot) = report.lock() {
                            *slot = Some(err.to_string());
                        }
                        let _ = sender.close();
                        return Err(err);
                    }
                }
            }
        });

        Ok(Self {
            handle,
            recv: Some(recv),
            file_sink_handles: vec![],
        })
    }

    pub fn receiver(&self) -> Receiver<ExecLogEntry> {
        self.recv
            .as_ref()
            .expect("receiver() called after join()")
            .clone()
    }

    /// Take ownership of a decoded-file sink worker thread. Its lifecycle is
    /// tied to this stream — `join()` will wait for it and propagate write
    /// errors so a truncated artifact doesn't slip past as success.
    pub fn attach_file_sink(&mut self, handle: JoinHandle<SinkOutcome>) {
        self.file_sink_handles.push(handle);
    }

    /// Wait for the execlog stream to finish.
    ///
    /// Drops the struct's `recv` clone so that if no external subscriber exists
    /// the first `try_send` returns `Closed` and remaining bytes are drained
    /// without proto decoding. Then waits for the reader thread and every
    /// attached file-sink writer, surfacing the first write error if any.
    pub fn join(mut self) -> Result<(), ExecLogStreamError> {
        self.recv.take();
        let reader_result = self.handle.join().expect("join error");
        for h in self.file_sink_handles {
            match h.join() {
                Ok(Ok(())) => {}
                Ok(Err(e)) => {
                    return Err(ExecLogStreamError::IO(std::io::Error::other(e.last_error)));
                }
                Err(_) => {
                    return Err(ExecLogStreamError::IO(std::io::Error::other(
                        "execlog file sink panicked",
                    )));
                }
            }
        }
        reader_result
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use axl_proto::tools::protos::exec_log_entry;

    /// Write `entries` as a compact execution log: one zstd frame over
    /// varint-length-prefixed `ExecLogEntry` messages, which is the format
    /// `--execution_log_compact_file` produces.
    fn write_log(path: &std::path::Path, entries: &[ExecLogEntry]) {
        let file = File::create(path).unwrap();
        let mut encoder = zstd::Encoder::new(file, 0).unwrap();
        for entry in entries {
            encoder
                .write_all(&entry.encode_length_delimited_to_vec())
                .unwrap();
        }
        encoder.finish().unwrap().flush().unwrap();
    }

    fn file_entry(id: u32, path: &str) -> ExecLogEntry {
        ExecLogEntry {
            id,
            r#type: Some(exec_log_entry::Type::File(exec_log_entry::File {
                path: path.to_string(),
                digest: None,
            })),
        }
    }

    fn drain(path: PathBuf) -> (Vec<ExecLogEntry>, Option<String>) {
        let failure: DecodeFailure = Default::default();
        let stream = ExecLogStream::spawn_from_path(path, failure.clone()).unwrap();
        let recv = stream.receiver();
        // Drop the stream's own receiver clone before consuming, exactly as the
        // Starlark `read()` does. In fibre's SPMC every clone is an independent
        // subscriber the sender must not lap, so an unconsumed one deadlocks the
        // producer on any log longer than the channel bound.
        drop(stream);
        let mut out = vec![];
        while let Ok(entry) = recv.recv() {
            out.push(entry);
        }
        // The reader thread records the failure before closing the channel, so
        // by the time recv() reports disconnection the slot is already set.
        let err = failure.lock().unwrap().clone();
        (out, err)
    }

    #[test]
    fn reads_every_entry_of_a_complete_log() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("log.binpb.zst");
        let entries: Vec<_> = (1..=250)
            .map(|i| file_entry(i, &format!("f{i}.txt")))
            .collect();
        write_log(&path, &entries);

        let (got, err) = drain(path);
        assert_eq!(err, None, "a complete log should not report a failure");
        assert_eq!(got.len(), 250);
        assert_eq!(got[0].id, 1);
        assert_eq!(got[249].id, 250);
    }

    #[test]
    fn rejects_a_file_that_is_not_zstd() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("not-a-log.zst");
        std::fs::write(&path, b"this is plainly not a zstd frame").unwrap();

        let failure: DecodeFailure = Default::default();
        let err = ExecLogStream::spawn_from_path(path, failure).unwrap_err();
        assert!(
            err.to_string().contains("is not a zstd frame"),
            "expected the magic-number check to name the problem, got: {err}"
        );
    }

    #[test]
    fn rejects_a_file_too_short_to_have_a_header() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("tiny.zst");
        std::fs::write(&path, b"ab").unwrap();

        let failure: DecodeFailure = Default::default();
        let err = ExecLogStream::spawn_from_path(path, failure).unwrap_err();
        assert!(
            err.to_string().contains("too short"),
            "expected a length complaint, got: {err}"
        );
    }

    #[test]
    fn a_missing_file_fails_on_the_calling_thread() {
        let failure: DecodeFailure = Default::default();
        let err =
            ExecLogStream::spawn_from_path(PathBuf::from("/definitely/not/here.zst"), failure)
                .unwrap_err();
        assert_eq!(err.kind(), io::ErrorKind::NotFound);
    }

    #[test]
    fn a_truncated_log_yields_a_prefix_rather_than_every_entry() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("log.binpb.zst");
        let entries: Vec<_> = (1..=200)
            .map(|i| file_entry(i, &format!("f{i}.txt")))
            .collect();
        write_log(&path, &entries);

        // Lop off the tail, keeping the header so the magic check still passes.
        let bytes = std::fs::read(&path).unwrap();
        let truncated = dir.path().join("truncated.binpb.zst");
        std::fs::write(&truncated, &bytes[..bytes.len() / 2]).unwrap();

        let (got, _) = drain(truncated);
        assert!(
            got.len() < 200,
            "expected fewer than every entry back from a truncated log, got {}",
            got.len()
        );
    }

    #[test]
    fn a_corrupt_frame_reports_why_it_stopped() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("log.binpb.zst");
        let entries: Vec<_> = (1..=2000)
            .map(|i| {
                file_entry(
                    i,
                    &format!("some/deep/path/to/a/source/file/number/{i}.txt"),
                )
            })
            .collect();
        write_log(&path, &entries);

        // Flip bytes in the middle of the compressed payload. Unlike truncation,
        // which can land on a block boundary and look like a clean end, this is
        // always a frame zstd refuses, so it pins the reporting path itself.
        let mut bytes = std::fs::read(&path).unwrap();
        let middle = bytes.len() / 2;
        for b in &mut bytes[middle..middle + 32] {
            *b ^= 0xFF;
        }
        let corrupt = dir.path().join("corrupt.binpb.zst");
        std::fs::write(&corrupt, &bytes).unwrap();

        let (got, err) = drain(corrupt);
        assert!(
            err.is_some(),
            "a corrupt frame must set the failure slot, or a diff silently compares \
             a prefix; got {} entries and no error",
            got.len()
        );
        assert!(
            got.len() < 2000,
            "expected the stream to stop short, got every entry back"
        );
    }
}
