use axl_proto::tools::protos::ExecLogEntry;
use fibre::spmc::{Receiver, bounded};
use fibre::{CloseError, SendError};
use prost::Message;
use std::fmt::Debug;
use std::fs::File;
use std::io;
use std::io::{BufWriter, Read, Write};
use std::path::PathBuf;
use std::thread::JoinHandle;
use std::{env, thread};

use crate::delimited;
use crate::fanout::SinkOutcome;
use crate::stream::file as followed;
use crate::stream::pid_pair::PidPair;
use thiserror::Error;
use zstd::Decoder;

/// One `CompactFile` copy the reader writes. A copy that cannot be opened,
/// written or flushed stops being written and keeps why, so the other copies and
/// the decoded stream go on and [`ExecLogStream::join`] reports it.
struct Tee {
    path: String,
    /// `None` once failed or finished.
    out: Option<BufWriter<File>>,
    error: Option<String>,
}

impl Tee {
    fn open(path: String) -> Self {
        let (out, error) = match File::create(&path) {
            Ok(f) => (Some(BufWriter::new(f)), None),
            Err(e) => (None, Some(format!("cannot open: {e}"))),
        };
        Self { path, out, error }
    }

    fn write(&mut self, bytes: &[u8]) {
        if let Some(Err(e)) = self.out.as_mut().map(|out| out.write_all(bytes)) {
            self.out = None;
            self.error = Some(format!("write failed: {e}"));
        }
    }

    /// Flush, and say why the copy is incomplete if it is.
    fn finish(&mut self) -> io::Result<()> {
        if let Some(Err(e)) = self.out.take().map(|mut out| out.flush()) {
            self.error = Some(format!("flush failed: {e}"));
        }
        match &self.error {
            Some(why) => Err(io::Error::other(format!(
                "execution log copy {}: {why}",
                self.path
            ))),
            None => Ok(()),
        }
    }
}

/// Wraps a `Read` source and tees every byte read to the [`Tee`]s, so they
/// capture the raw bytes without a second pass.
struct MultiTeeReader<R: Read> {
    inner: R,
    tees: Vec<Tee>,
}

impl<R: Read> Read for MultiTeeReader<R> {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        let n = self.inner.read(buf)?;
        for tee in &mut self.tees {
            tee.write(&buf[..n]);
        }
        Ok(n)
    }
}

impl<R: Read> MultiTeeReader<R> {
    /// Finish every copy, even after one fails, returning the first failure.
    fn finish(&mut self) -> io::Result<()> {
        let mut first = Ok(());
        for tee in &mut self.tees {
            let result = tee.finish();
            if first.is_ok() {
                first = result;
            }
        }
        first
    }
}

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
pub fn channel_capacity() -> usize {
    CHANNEL_CAPACITY
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
    /// The thread streams the file as Bazel writes it using
    /// [`crate::stream::file::File`], which waits for the file to exist and for
    /// more bytes while Bazel holds it open, and says when the stream is over. It
    /// self-terminates when Bazel closes the file.
    ///
    /// Unlike the BES FIFO, a regular file loses nothing by starting late: bytes
    /// written before the reader opens are still there to be read.
    ///
    /// ## Send strategy
    ///
    /// Entries are sent with the blocking [`fibre::spmc::Sender::send`], so every
    /// subscriber sees every entry: a decoded `File` sink, which would otherwise
    /// write an incomplete file; an
    /// `ExecLogIter` handle backing
    /// `BazelTrait.exec_log_event` hooks; and the stream's own subscriber, which
    /// `build.execution_logs()` takes — each of which would otherwise miss entries
    /// with nothing to say so. The reader waits for the slowest
    /// subscriber; `build.wait()` releases the AXL-side consumers and then joins,
    /// and `join()`'s first act is to release the stream's own — so one that stops
    /// reading cannot hold the reader forever.
    ///
    /// A send that fails means every subscriber is gone, which is not an error:
    /// there is nobody left to miss the remaining entries, and failing here would
    /// fail an otherwise good build. The thread drains the rest of the bytes
    /// without paying to decode them, and that failure is also what ends a reader
    /// parked against a subscriber nobody drained.
    ///
    /// The regular file is what makes blocking affordable: Bazel writes the file
    /// and this thread tails it, so a slow consumer parks *this* thread and never
    /// Bazel. Back-pressure shows up as a longer `join()` at the end of the build,
    /// not as a slower build. A regular file rather than a FIFO, deliberately:
    /// Bazel checksums the compact log after writing it and cannot seek back on a
    /// pipe, which stalls it mid-build and deadlocks against a BEP FIFO —
    /// https://github.com/bazelbuild/bazel/issues/28800.
    ///
    /// `CompactFile` sinks are unaffected — raw bytes are tee'd by
    /// [`MultiTeeReader`] before decoding. A copy that fails stops being written
    /// while everything else gets the whole log, and the failure is the thread's
    /// result.
    pub fn spawn_with_file(
        path: PathBuf,
        server_pid: u32,
        client_pid: u32,
        compact_sink_paths: Vec<String>,
    ) -> io::Result<Self> {
        let (mut sender, recv) = bounded::<ExecLogEntry>(CHANNEL_CAPACITY);
        let gate = PidPair {
            client: client_pid,
            server: Some(server_pid),
        }
        .gate(&path);
        let mut file = followed::File::follow(path, gate);
        let handle = thread::spawn(move || {
            let mut buf: Vec<u8> = Vec::with_capacity(1024 * 5);

            // Bazel exited without ever writing an execution log: an empty
            // stream, not a failure, since the build's own exit code is the thing
            // the caller wants to see.
            //
            // Whether that is worth saying out loud depends on the exit code,
            // which this thread does not have — so it reports the fact and
            // `join()`'s caller decides. See [`ReaderOutcome::NeverAppeared`].
            if !file.appear() {
                if let Some(crate::stream::Done::Broken { error, .. }) = file.take_done() {
                    return Err(error.into());
                }
                sender.close()?;
                return Ok(ReaderOutcome::NeverAppeared);
            }
            let out_raw = MultiTeeReader {
                inner: file,
                tees: compact_sink_paths.into_iter().map(Tee::open).collect(),
            };
            let mut out_raw = Decoder::new(out_raw)?;

            let mut has_readers = true;

            loop {
                match delimited::next_frame(&mut out_raw, &mut buf) {
                    Ok(Some(header)) => {
                        if has_readers {
                            let entry = ExecLogEntry::decode(&buf[..header.len])?;
                            has_readers = sender.send(entry).is_ok();
                        }
                    }
                    // The end: between two entries, or the stream stopped under
                    // the decoder (Bazel killed, its client gone, the file
                    // failing), which leaves a frame incomplete. The entries
                    // before it were good; how it ended is the file's to say.
                    Ok(None) => break,
                    Err(_) if out_raw.get_ref().get_ref().inner.done().is_some() => break,
                    Err(err) => return Err(err.into()),
                }
            }
            sender.close()?;
            out_raw.get_mut().get_mut().finish()?;
            Ok(ReaderOutcome::Read)
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

    /// A copy that cannot open costs only itself: the reader still passes every
    /// byte through, the good copy gets them all, and `finish` names the bad one.
    #[test]
    fn a_tee_that_cannot_open_fails_alone() {
        let dir = tempfile::tempdir().unwrap();
        let good = dir.path().join("good.zstd");
        let bad = dir.path().join("missing-dir").join("bad.zstd");
        let bytes = b"compact log bytes".to_vec();
        let mut tee = MultiTeeReader {
            inner: bytes.as_slice(),
            tees: [&bad, &good]
                .map(|p| Tee::open(p.to_str().unwrap().to_string()))
                .into(),
        };

        let mut read = vec![];
        tee.read_to_end(&mut read).unwrap();
        let err = tee.finish().unwrap_err().to_string();

        assert_eq!(read, bytes);
        assert_eq!(std::fs::read(&good).unwrap(), bytes);
        assert!(
            err.contains(bad.to_str().unwrap()) && err.contains("cannot open"),
            "{err}"
        );
    }

    #[test]
    fn tees_that_all_open_finish_cleanly() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("copy.zstd");
        let mut tee = MultiTeeReader {
            inner: &b"abc"[..],
            tees: vec![Tee::open(path.to_str().unwrap().to_string())],
        };
        std::io::copy(&mut tee, &mut std::io::sink()).unwrap();
        tee.finish().unwrap();
        assert_eq!(std::fs::read(&path).unwrap(), b"abc");
    }

    /// Why the stream must not keep an unread subscriber once real ones exist.
    ///
    /// In fibre's broadcast ring each subscriber has its own tail and the producer
    /// may not lap the slowest. One that nobody reads sits at entry zero forever,
    /// so the ring reads as full from the producer's side however diligently every
    /// other subscriber drains — capping them all at the capacity. Blocking sends
    /// then park, which the consumer cannot see.
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
