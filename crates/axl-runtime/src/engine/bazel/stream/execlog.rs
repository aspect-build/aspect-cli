use axl_proto::tools::protos::ExecLogEntry;
use fibre::spmc::{Receiver, Sender, bounded};
use fibre::{CloseError, SendError, TrySendError};
use prost::Message;
use std::fmt::Debug;
use std::fs::File;
use std::io;
use std::io::{BufWriter, Read, Write};
use std::path::PathBuf;
use std::thread::JoinHandle;
use std::{env, thread};

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

/// Decoded entries buffered between the reader thread and its subscribers.
///
/// A broadcast ring: the producer may not lap its slowest subscriber, so this is
/// also how far ahead of the slowest consumer the reader may get before it either
/// blocks or starts dropping, depending on [`ExecLogStream::spawn`]'s `lossless`.
const CHANNEL_CAPACITY: usize = 1000;

/// [`CHANNEL_CAPACITY`], for tests that assert on behaviour at the boundary
/// rather than restating the number.
#[cfg(test)]
pub fn channel_capacity() -> usize {
    CHANNEL_CAPACITY
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

/// Hand one decoded entry to the stream's subscribers, by whichever strategy
/// `lossless` selects (see [`ExecLogStream::spawn`]).
///
/// Returns whether any subscriber remains. `false` means every one is gone — the
/// file-sink threads finished and `build.wait()` released the iterator handles —
/// so the caller can drain the rest of the bytes without paying to decode them.
/// On the blocking path that is also why a closed channel is not an error: there
/// is nobody left to miss the remaining entries, and failing here would fail an
/// otherwise good build.
fn forward(sender: &Sender<ExecLogEntry>, entry: ExecLogEntry, lossless: bool) -> bool {
    if lossless {
        return sender.send(entry).is_ok();
    }
    match sender.try_send(entry) {
        // Full: the consumer is behind and the entry is dropped by design.
        Ok(()) | Err(TrySendError::Sent(_)) | Err(TrySendError::Full(_)) => true,
        Err(TrySendError::Closed(_)) => false,
    }
}

#[derive(Debug)]
pub struct ExecLogStream {
    handle: JoinHandle<Result<(), ExecLogStreamError>>,
    /// The one subscriber the stream starts with, and the only thing a further
    /// subscriber can be cloned from — so once it is gone, nothing can join.
    ///
    /// Nobody reads it, which is why it is an `Option` to be given away or
    /// dropped as soon as the real consumers are wired: in fibre's broadcast ring
    /// the producer may not lap its slowest subscriber, and an unread one is a
    /// tail stuck at entry zero. Left in place it caps every other consumer at
    /// [`CHANNEL_CAPACITY`] — blocking the producer there, or silently dropping
    /// past it. Gone with no consumer in its place, the first send reports
    /// `Closed` and the reader skips decoding entirely, which is what `join()`
    /// relies on.
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
        lossless: bool,
    ) -> io::Result<(PathBuf, Self)> {
        let out = env::temp_dir().join(format!("execlog-out-{}.bin", uuid::Uuid::new_v4()));
        let stream = Self::spawn(out.clone(), pid, compact_sink_paths, lossless)?;
        Ok((out, stream))
    }

    /// Spawn the execlog reader thread.
    ///
    /// ## Send strategy
    ///
    /// `lossless` controls how decoded entries are sent to the channel:
    ///
    /// - `true` — blocking [`Sender::send`]: every subscriber sees every entry. Set
    ///   when one of them requires that — a decoded `File` sink, which would otherwise
    ///   write an incomplete file, or an [`ExecLogIter`](super::super::iter::ExecLogIter)
    ///   handle backing `BazelTrait.exec_log_event` hooks, which would otherwise miss
    ///   entries with nothing to say so. The reader waits for the slowest subscriber;
    ///   `build.wait()` releases the handles before joining it, so a consumer that
    ///   stops reading cannot hold the reader forever.
    ///
    /// - `false` — non-blocking [`Sender::try_send`]: entries past
    ///   [`CHANNEL_CAPACITY`] ahead of the slowest subscriber are dropped rather than
    ///   stalling the reader. The strategy for `execution_logs()`, which is handed out
    ///   after the spawn and so cannot be counted here.
    ///
    /// `CompactFile` sinks are unaffected by this flag — raw bytes are always tee'd
    /// by `MultiTeeReader` before decoding.
    pub fn spawn(
        path: PathBuf,
        pid: u32,
        compact_sink_paths: Vec<String>,
        lossless: bool,
    ) -> io::Result<Self> {
        let (mut sender, recv) = bounded::<ExecLogEntry>(CHANNEL_CAPACITY);
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

            let mut has_readers = true;

            let mut read = || -> Result<(), ExecLogStreamError> {
                // varint size can be somewhere between 1 to 10 bytes.
                let (size, _) = read_varint(&mut out_raw)?;
                if size > buf.len() {
                    buf.resize(size, 0);
                }

                out_raw.read_exact(&mut buf[0..size])?;

                if has_readers {
                    let entry = ExecLogEntry::decode(&buf[0..size])?;
                    has_readers = forward(&sender, entry, lossless);
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

    /// Mint the path Bazel will write `--execution_log_compact_file` to, without
    /// starting a reader. Pass `Some(path)` to reuse an existing sink path (e.g. a
    /// `CompactFile` sink, so Bazel writes straight to the caller's destination with
    /// no tee step); `None` names a UUID temp file. Nothing is created on disk —
    /// Bazel writes a regular file, so there is no inode to reserve.
    ///
    /// Pair with [`spawn_with_file`](Self::spawn_with_file) once the caller has the
    /// bazel client pid in hand.
    pub fn reserve_path(out_path: Option<PathBuf>) -> PathBuf {
        out_path.unwrap_or_else(|| {
            env::temp_dir().join(format!("execlog-out-{}.bin", uuid::Uuid::new_v4()))
        })
    }

    /// Spawn the execlog reader thread for a regular file at `path`, as minted by
    /// [`reserve_path`](Self::reserve_path).
    ///
    /// `server_pid` is the Bazel daemon: the process that holds the file open while
    /// it writes, so it answers "are more bytes coming?".
    ///
    /// `client_pid` is the spawned bazel client — the per-invocation pid whose death
    /// means no execution log is coming at all. It has to be asked separately,
    /// because a Bazel that rejects its command line exits before writing anything
    /// and leaves the daemon idling behind it for `--max_idle_secs`; keyed to
    /// `server_pid` alone the open below waits out that whole idle period.
    ///
    /// The thread streams the file as Bazel writes it using [`galvanize::StreamingFile`],
    /// which busy-polls for file existence at open time and retries reads while Bazel
    /// holds the file open. It self-terminates when Bazel closes the file.
    ///
    /// Unlike the BES FIFO, a regular file loses nothing by starting late: bytes
    /// written before the reader opens are still there to be read.
    ///
    /// `lossless` picks the send strategy, as described on [`spawn`](Self::spawn).
    /// This is the production path, and the regular file is why `true` is affordable
    /// here: Bazel writes the file and this thread tails it, so a slow consumer
    /// parks *this* thread and never Bazel. Back-pressure therefore shows up as a
    /// longer `join()` at the end of the build, not as a slower build.
    pub fn spawn_with_file(
        path: PathBuf,
        server_pid: u32,
        client_pid: u32,
        compact_sink_paths: Vec<String>,
        lossless: bool,
    ) -> io::Result<Self> {
        let (mut sender, recv) = bounded::<ExecLogEntry>(CHANNEL_CAPACITY);
        let handle = thread::spawn(move || {
            let mut buf: Vec<u8> = Vec::with_capacity(1024 * 5);
            // 10 is the maximum size of a varint so start with that size.
            buf.resize(10, 0);

            let out_raw = match galvanize::StreamingFile::open(path.clone(), server_pid, client_pid)
            {
                Ok(f) => f,
                // Bazel exited without ever writing an execution log — a rejected
                // command line, say. An empty stream, not a failure: the build's own
                // exit code is the thing the caller wants to see.
                Err(err) if err.kind() == io::ErrorKind::BrokenPipe => {
                    sender.close()?;
                    return Ok(());
                }
                Err(err) => return Err(err.into()),
            };
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

            let mut has_readers = true;

            let mut read = || -> Result<(), ExecLogStreamError> {
                let (size, _) = read_varint(&mut out_raw)?;
                if size > buf.len() {
                    buf.resize(size, 0);
                }

                out_raw.read_exact(&mut buf[0..size])?;

                if has_readers {
                    let entry = ExecLogEntry::decode(&buf[0..size])?;
                    has_readers = forward(&sender, entry, lossless);
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

        Ok(Self {
            handle,
            recv: Some(recv),
            file_sink_handles: vec![],
        })
    }

    /// A further subscriber to the decoded stream, cloned from the stream's own.
    /// `None` once that one has been given away or dropped
    /// ([`take_initial_subscriber`](Self::take_initial_subscriber), `join()`).
    ///
    /// Only for a consumer that will actually drain it — see the `recv` field.
    pub fn receiver(&self) -> Option<Receiver<ExecLogEntry>> {
        self.recv.as_ref().cloned()
    }

    /// Give the stream's own subscriber away, leaving it with none.
    ///
    /// Two callers, both with the same requirement — that the stream not keep an
    /// unread subscriber once the real consumers are wired (see the `recv`
    /// field). `build.execution_logs()` takes it *as* its consumer, so the one
    /// subscriber is the one being drained; `Build::spawn` drops the value, once
    /// the sinks and iterator handles have cloned what they need.
    ///
    /// `None` means it has already gone, so no further consumer can be added.
    pub fn take_initial_subscriber(&mut self) -> Option<Receiver<ExecLogEntry>> {
        self.recv.take()
    }

    /// Take ownership of a decoded-file sink worker thread. Its lifecycle is
    /// tied to this stream — `join()` will wait for it and propagate write
    /// errors so a truncated artifact doesn't slip past as success.
    pub fn attach_file_sink(&mut self, handle: JoinHandle<SinkOutcome>) {
        self.file_sink_handles.push(handle);
    }

    /// Wait for the execlog stream to finish.
    ///
    /// Releases the stream's own subscriber if it still holds one, so that with no
    /// consumer left the reader stops decoding and drains. Then waits for the
    /// reader thread and every attached file-sink writer, surfacing the first
    /// write error if any.
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

    fn entry(id: u32) -> ExecLogEntry {
        ExecLogEntry { id, r#type: None }
    }

    /// Why the stream must not keep an unread subscriber once real ones exist.
    ///
    /// In fibre's broadcast ring each subscriber has its own tail and the producer
    /// may not lap the slowest. One that nobody reads sits at entry zero forever,
    /// so the ring reads as full from the producer's side however diligently every
    /// other subscriber drains — capping them all at the capacity. Blocking sends
    /// then park and lossy ones drop, neither of which the consumer can see.
    #[test]
    fn an_unread_subscriber_clone_stops_the_producer_once_the_ring_fills() {
        // A small ring: the behaviour is in the tails, not the size.
        const CAPACITY: u32 = 4;
        let (sender, kept) = bounded::<ExecLogEntry>(CAPACITY as usize);
        let drained = kept.clone();

        for i in 0..CAPACITY {
            sender.try_send(entry(i)).expect("the ring has room");
        }
        for i in 0..CAPACITY {
            assert_eq!(drained.recv().expect("an entry").id, i);
        }

        // `drained` is caught up, but `kept` never read anything, so from the
        // producer's side the ring is still full.
        assert!(
            matches!(sender.try_send(entry(99)), Err(TrySendError::Full(_))),
            "an unread clone should hold the ring full even though every other \
             subscriber is caught up",
        );

        drop(kept);
        sender
            .try_send(entry(99))
            .expect("dropping the unread clone must free the ring");
        assert_eq!(drained.recv().expect("an entry").id, 99);
    }
}
