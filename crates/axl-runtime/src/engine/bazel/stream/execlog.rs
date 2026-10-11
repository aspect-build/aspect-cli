use axl_proto::tools::protos::ExecLogEntry;
use fibre::spmc::{Receiver, Sender, bounded};
use fibre::{CloseError, SendError};
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

/// What the reader thread found at the execution log's path.
///
/// Bazel creates the compact log once a command gets past target-pattern parsing
/// and package loading — including for a build that runs no actions at all, which
/// still leaves an empty zstd frame (28 bytes on Bazel 7, 66 on 8 and 9). A
/// pattern that resolves and matches nothing (`//...` in an empty workspace,
/// `//pkg:all`) is one of those: it exits 0 and writes the empty frame. Bazel does
/// *not* create one for a pattern that does not resolve — no such package or
/// target — an unparseable BUILD file or a rejected flag, and every one of those
/// exits nonzero.
///
/// So the absence of a log is only remarkable on an invocation that *succeeded*,
/// and that is the one the caller is asked to warn about: a successful build whose
/// log never appeared is the shape a dead nominated holder produces, where every
/// hook fires zero times and nothing says so. On a failed build the absence is
/// ordinary and Bazel has already printed the reason, so a warning there would
/// only crowd it out.
///
/// The outcome reaches a caller only through `join()`, and `Build` joins the
/// stream from `wait()` alone — there is no `Drop` path and `try_wait` does
/// not join — so a task that never calls `wait()` never learns it, and no warning
/// is printed. Every wired site calls `wait()`, so this is a note about where the
/// verdict lives rather than a gap.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ReaderOutcome {
    /// The log appeared and was read to the end of the stream.
    Read,
    /// The invocation ended before the log appeared.
    NeverAppeared,
}

/// Decoded entries buffered between the reader thread and its subscribers.
///
/// A broadcast ring: the producer may not lap its slowest subscriber, so this is
/// how far ahead of the slowest consumer the reader may get before it blocks. It
/// is the whole of the back-pressure budget — nothing is dropped past it — so it
/// trades memory for how long a slow consumer can be slow without costing the
/// reader a context switch per entry.
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

/// Hand one decoded entry to the stream's subscribers, blocking until the
/// slowest has room.
///
/// Returns whether any subscriber remains. `false` means every one is gone — the
/// file-sink threads finished, and `build.wait()` released the AXL-side consumers
/// and then, in `join()`, the stream's own subscriber — so the caller can drain
/// the rest of the bytes without paying to decode them. That is also why a closed
/// channel is not an error: there is nobody left to miss the remaining entries,
/// and failing here would fail an otherwise good build. It is what ends a reader
/// parked against a subscriber nobody drained.
fn forward(sender: &Sender<ExecLogEntry>, entry: ExecLogEntry) -> bool {
    sender.send(entry).is_ok()
}

#[derive(Debug)]
pub struct ExecLogStream {
    handle: JoinHandle<Result<ReaderOutcome, ExecLogStreamError>>,
    /// The one subscriber the stream starts with, and the only thing a further
    /// subscriber can be cloned from — so once it is gone, nothing can join.
    ///
    /// It is an `Option` because it has to be given away or dropped as soon as
    /// other consumers are wired: in fibre's broadcast ring the producer may not
    /// lap its slowest subscriber, and one nobody reads is a tail stuck at entry
    /// zero. Left in place beside others it caps them all at
    /// [`CHANNEL_CAPACITY`], blocking the producer there. Gone with no consumer
    /// in its place, the first send reports `Closed` and the reader skips decoding
    /// entirely, which is what `join()` relies on.
    ///
    /// A stream that named no consumer keeps it instead, because
    /// `build.execution_logs()` may still claim it. Nothing drains it until
    /// something does, so until then the producer sits at [`CHANNEL_CAPACITY`] —
    /// which costs the build nothing (Bazel writes a regular file that this
    /// process tails) and is bounded by `join()` releasing it.
    recv: Option<Receiver<ExecLogEntry>>,
    /// Decoded-file sink writer threads owned by this stream — joined in `join()`.
    file_sink_handles: Vec<JoinHandle<SinkOutcome>>,
}

impl ExecLogStream {
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
    /// ## Send strategy
    ///
    /// Entries are sent with the blocking [`Sender::send`], so every subscriber
    /// sees every entry: a decoded `File` sink, which would otherwise write an
    /// incomplete file; an [`ExecLogIter`](super::super::iter::ExecLogIter) handle
    /// backing `BazelTrait.exec_log_event` hooks; and the stream's own subscriber,
    /// which `build.execution_logs()` takes — each of which would otherwise miss
    /// entries with nothing to say so. The reader waits for the slowest
    /// subscriber; `build.wait()` releases the AXL-side consumers, and then the
    /// stream's own, before joining it, so one that stops reading cannot hold the
    /// reader forever.
    ///
    /// The regular file is what makes blocking affordable: Bazel writes the file
    /// and this thread tails it, so a slow consumer parks *this* thread and never
    /// Bazel. Back-pressure shows up as a longer `join()` at the end of the build,
    /// not as a slower build.
    ///
    /// `CompactFile` sinks are unaffected — raw bytes are tee'd by
    /// [`MultiTeeReader`] before decoding.
    pub fn spawn_with_file(
        path: PathBuf,
        server_pid: u32,
        client_pid: u32,
        compact_sink_paths: Vec<String>,
    ) -> io::Result<Self> {
        let (mut sender, recv) = bounded::<ExecLogEntry>(CHANNEL_CAPACITY);
        let handle = thread::spawn(move || {
            let mut buf: Vec<u8> = Vec::with_capacity(1024 * 5);
            // 10 is the maximum size of a varint so start with that size.
            buf.resize(10, 0);

            let out_raw = match galvanize::StreamingFile::open(path.clone(), server_pid, client_pid)
            {
                Ok(f) => f,
                // Bazel exited without ever writing an execution log. An empty
                // stream, not a failure: the build's own exit code is the thing the
                // caller wants to see.
                //
                // Whether that is worth saying out loud depends on the exit code,
                // which this thread does not have — so it reports the fact and
                // `join()`'s caller decides. See [`ReaderOutcome::NeverAppeared`].
                Err(err) if err.kind() == io::ErrorKind::BrokenPipe => {
                    sender.close()?;
                    return Ok(ReaderOutcome::NeverAppeared);
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
                    has_readers = forward(&sender, entry);
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
                        return Ok(ReaderOutcome::Read);
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

    /// Join the reader and every file sink, reporting what the reader found at
    /// the log's path. See [`ReaderOutcome`] for what a caller does with it.
    ///
    /// Releases the stream's own subscriber if it still holds one, so that with no
    /// consumer left the reader stops decoding and drains. Then waits for the
    /// reader thread and every attached file-sink writer, surfacing the first
    /// write error if any.
    pub fn join(mut self) -> Result<ReaderOutcome, ExecLogStreamError> {
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
    use fibre::TrySendError;

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
