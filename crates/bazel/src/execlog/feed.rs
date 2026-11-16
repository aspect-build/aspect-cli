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

/// One `CompactFile` copy the reader writes. A copy that cannot be created,
/// written or flushed stops being written (whatever reached the file stays) and
/// keeps why, so the other copies and the decoded stream are unaffected and
/// [`ExecLogStream::join`] reports it.
struct Tee {
    path: String,
    /// `None` once failed or finished.
    out: Option<Box<dyn Write + Send>>,
    error: Option<String>,
}

impl Tee {
    fn open(path: String) -> Self {
        match File::create(&path) {
            Ok(f) => Self::new(path, Box::new(BufWriter::new(f))),
            Err(e) => Self {
                error: Some(format!("ExecLog: failed to create '{path}': {e}")),
                path,
                out: None,
            },
        }
    }

    fn new(path: String, out: Box<dyn Write + Send>) -> Self {
        Self {
            path,
            out: Some(out),
            error: None,
        }
    }

    fn write(&mut self, bytes: &[u8]) {
        let Some(out) = self.out.as_mut() else { return };
        if let Err(e) = out.write_all(bytes) {
            self.out = None;
            self.error = Some(format!("ExecLog: write to '{}' failed: {e}", self.path));
        }
    }

    /// Start the copy over, empty, as bazel starts its own log over for a
    /// retried attempt. A copy that has failed stays failed.
    fn restart(&mut self) {
        if self.out.is_none() || self.finish().is_some() {
            return;
        }
        *self = Tee::open(std::mem::take(&mut self.path));
    }

    /// Flush the copy; returns why it is incomplete, if it is.
    fn finish(&mut self) -> Option<String> {
        if let Some(mut out) = self.out.take()
            && let Err(e) = out.flush()
        {
            self.error = Some(format!("ExecLog: flush of '{}' failed: {e}", self.path));
        }
        self.error.clone()
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
    fn restart(&mut self) {
        self.tees.iter_mut().for_each(Tee::restart);
    }

    /// Finish every copy, failing with each failed copy's reason.
    fn finish(&mut self) -> io::Result<()> {
        let failures: Vec<String> = self.tees.iter_mut().filter_map(Tee::finish).collect();
        if failures.is_empty() {
            Ok(())
        } else {
            Err(io::Error::other(failures.join("; ")))
        }
    }
}

/// Fail when the followed log at `path` ended `done` by breaking, naming the
/// path and how far it got. A read error ends the file's stream like its end
/// does, so without this a log cut short would pass as a complete log.
fn broken_read(done: Option<&crate::stream::Done>, path: &std::path::Path) -> io::Result<()> {
    let path = path.display();
    match done {
        Some(crate::stream::Done::Broken { bytes: 0, error }) => Err(io::Error::other(format!(
            "ExecLog: reading '{path}' failed: {error}"
        ))),
        Some(crate::stream::Done::Broken { bytes, error }) => Err(io::Error::other(format!(
            "ExecLog: reading '{path}' failed after {bytes} bytes: {error}"
        ))),
        _ => Ok(()),
    }
}

/// The decoded stream over the followed file, as the reader thread holds it.
type Decoding<'a> = Decoder<'a, io::BufReader<MultiTeeReader<followed::File>>>;

/// The file under `decoder` has said why it stopped, so a decode error is the
/// stream cut short, not corrupt.
fn decoder_done(decoder: &Option<Decoding<'_>>) -> bool {
    decoder
        .as_ref()
        .is_some_and(|d| d.get_ref().get_ref().inner.done().is_some())
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
    /// The log appeared and was read to the end of the stream. A read error
    /// fails the stream instead.
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
    /// It is an `Option` because it has to be dropped ([`seal`](Self::seal)) as
    /// soon as the other consumers are wired: in fibre's broadcast ring the
    /// producer may not lap its slowest subscriber, and one nobody reads is a
    /// tail stuck at entry zero. Left in place beside others it caps them all at
    /// [`CHANNEL_CAPACITY`], blocking the producer there. Gone with no consumer
    /// in its place, the first send reports `Closed` and the reader skips
    /// decoding entirely.
    recv: Option<Receiver<ExecLogEntry>>,
    /// Decoded-entry sink threads owned by this stream — joined in `join()`.
    sink_handles: Vec<JoinHandle<SinkOutcome>>,
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
    /// it writes, so once the client has exited it answers "are more bytes
    /// coming?".
    ///
    /// `client_pid` is the spawned bazel client — the per-invocation pid whose death
    /// means no execution log is coming at all. It has to be asked separately,
    /// because a Bazel that rejects its command line exits before writing anything
    /// and leaves the daemon idling behind it for `--max_idle_secs`; keyed to
    /// `server_pid` alone the open below waits out that whole idle period.
    ///
    /// The thread streams the file as Bazel writes it using
    /// [`crate::stream::file::File`], which waits for the file to exist and for
    /// more bytes while Bazel holds it open, and says when the stream is over:
    /// once Bazel has closed the file and the client has exited, or once the
    /// quiet window after the client's exit has passed (see [`crate::stream`]).
    /// A read error fails `join()`.
    ///
    /// A regular file loses nothing by starting late: bytes written before the
    /// reader opens are still there to be read.
    ///
    /// ## Send strategy
    ///
    /// Entries are sent with the blocking [`fibre::spmc::Sender::send`], so every
    /// subscriber sees every entry: a decoded `File` sink, which would otherwise
    /// write an incomplete file; the spawn telemetry sink; an `ExecLogIter`
    /// handle backing `BazelTrait.exec_log_event` hooks — each of which would
    /// otherwise miss entries with nothing to say so. The reader waits for the
    /// slowest subscriber. The stream's own is dropped once consumers are bound
    /// ([`seal`](Self::seal)) and `build.wait()` releases the AXL-side ones
    /// before joining, so one that stops reading cannot hold the reader forever.
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
    /// pipe, which stalls it mid-build —
    /// https://github.com/bazelbuild/bazel/issues/28800.
    ///
    /// `CompactFile` sinks are unaffected — raw bytes are tee'd by
    /// [`MultiTeeReader`] before decoding. A copy that cannot be created or
    /// written fails alone, leaving the other copies and the decoded stream
    /// unaffected, and [`join`](Self::join) reports the failure.
    ///
    /// A log bazel starts over in place, as it does for a retried attempt, is
    /// decoded again from its start, and every copy starts over with it.
    /// Decoded consumers get every attempt's entries, one after the other. The
    /// restart is noticed only if the reader reaches the old end before the new
    /// log grows past it: a reader parked by a slow consumer meanwhile reads on
    /// into the middle of the new log, which fails to decode.
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
        .gate();
        let mut file = followed::File::follow(path, gate);
        let handle = thread::spawn(move || {
            let mut buf: Vec<u8> = Vec::with_capacity(1024 * 5);

            // Bazel exited without ever writing an execution log: an empty
            // stream, not a failure, since the build's own exit code is the thing
            // the caller wants to see. A path that cannot be opened is a failure.
            //
            // Whether that is worth saying out loud depends on the exit code,
            // which this thread does not have — so it reports the fact and
            // `join()`'s caller decides. See [`ReaderOutcome::NeverAppeared`].
            if !file.appear() {
                let broken = broken_read(file.done(), file.path());
                let closed = sender.close();
                broken?;
                closed?;
                return Ok(ReaderOutcome::NeverAppeared);
            }
            let out_raw = MultiTeeReader {
                inner: file,
                tees: compact_sink_paths.into_iter().map(Tee::open).collect(),
            };
            // An `Option` so a restart can take the reader back from it.
            let mut out_raw = Some(Decoder::new(out_raw)?);

            let read = (|| -> Result<ReaderOutcome, ExecLogStreamError> {
                let mut has_readers = true;
                loop {
                    let decoder = out_raw.as_mut().expect("put back after a restart");
                    match delimited::next_frame(decoder, &mut buf) {
                        Ok(Some(header)) => {
                            if has_readers {
                                let entry = ExecLogEntry::decode(&buf[..header.len])?;
                                has_readers = sender.send(entry).is_ok();
                            }
                        }
                        // The end: between two entries, or the stream stopped
                        // under the decoder (Bazel killed, its client gone, the
                        // file failing), which leaves a frame incomplete. The
                        // entries before it were good; how it ended is the
                        // file's to say.
                        Ok(None) => break,
                        // Bazel started the log over for a retried attempt: a
                        // new zstd stream, so a new decoder, and the copies
                        // start over with the file.
                        Err(err) if followed::StartedOver::is(&err) => {
                            let decoder = out_raw.take().expect("taken only here");
                            let mut inner = decoder.finish().into_inner();
                            inner.restart();
                            out_raw = Some(Decoder::new(inner)?);
                        }
                        Err(_) if decoder_done(&out_raw) => break,
                        Err(err) => return Err(err.into()),
                    }
                }
                let decoder = out_raw.as_ref().expect("put back after a restart");
                let file = &decoder.get_ref().get_ref().inner;
                let broken = broken_read(file.done(), file.path());
                let closed = sender.close();
                broken?;
                closed?;
                Ok(ReaderOutcome::Read)
            })();
            // Every copy is finished however reading ended, so a failed one is
            // reported even beside a reader error.
            let copies = match out_raw.as_mut() {
                Some(decoder) => decoder.get_mut().get_mut().finish(),
                None => Ok(()),
            };
            combine(
                read,
                copies.err().map(|e| e.to_string()).into_iter().collect(),
            )
        });

        Ok(Self {
            handle,
            recv: Some(recv),
            sink_handles: vec![],
        })
    }

    /// A further subscriber to the decoded stream, cloned from the stream's own.
    /// `None` once that one has been dropped ([`seal`](Self::seal), `join()`).
    ///
    /// Only for a consumer that will actually drain it — see the `recv` field.
    pub fn receiver(&self) -> Option<Receiver<ExecLogEntry>> {
        self.recv.as_ref().cloned()
    }

    /// Drop the stream's own subscriber once every consumer has cloned one from
    /// [`receiver`](Self::receiver), so it cannot hold the ring full (see the
    /// `recv` field). No consumer can be added afterwards.
    pub fn seal(&mut self) {
        self.recv.take();
    }

    /// Take ownership of a decoded-entry sink thread. Its lifecycle is tied to
    /// this stream — `join()` waits for it and fails with its error or panic, so
    /// a decoded file sink's truncated artifact doesn't slip past as success.
    pub fn attach_sink(&mut self, handle: JoinHandle<SinkOutcome>) {
        self.sink_handles.push(handle);
    }

    /// Join the reader and every attached sink, reporting what the reader found
    /// at the log's path. See [`ReaderOutcome`] for what a caller does with it.
    ///
    /// Releases the stream's own subscriber if it still holds one, so that with no
    /// consumer left the reader stops decoding and drains. Then waits for the
    /// reader thread and every attached sink, failing with every
    /// error among them (see [`combine`]).
    pub fn join(mut self) -> Result<ReaderOutcome, ExecLogStreamError> {
        self.recv.take();
        let reader_result = self.handle.join().expect("join error");
        let sink_errors = self
            .sink_handles
            .into_iter()
            .filter_map(|h| match h.join() {
                Ok(Ok(())) => None,
                Ok(Err(e)) => Some(e.last_error),
                Err(_) => Some("execlog sink panicked".to_owned()),
            })
            .collect();
        combine(reader_result, sink_errors)
    }
}

/// `result`, failed with its own error and every one in `more`. A lone error
/// keeps its type. The reader uses it to add failed compact copies to how
/// reading ended, and `join` to add failed decoded sinks to the reader's.
fn combine(
    result: Result<ReaderOutcome, ExecLogStreamError>,
    more: Vec<String>,
) -> Result<ReaderOutcome, ExecLogStreamError> {
    if more.is_empty() {
        return result;
    }
    let mut errors = vec![];
    if let Err(e) = result {
        errors.push(match e {
            ExecLogStreamError::IO(io) => io.to_string(),
            other => other.to_string(),
        });
    }
    errors.extend(more);
    Err(ExecLogStreamError::IO(io::Error::other(errors.join("; "))))
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
            err.contains(bad.to_str().unwrap()) && err.contains("failed to create"),
            "{err}"
        );
    }

    /// A writer that accepts `budget` bytes, then fails every write or flush.
    struct Failing {
        budget: usize,
        fail_flush: bool,
    }

    impl Write for Failing {
        fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
            if buf.len() > self.budget {
                return Err(io::Error::other("disk full"));
            }
            self.budget -= buf.len();
            Ok(buf.len())
        }
        fn flush(&mut self) -> io::Result<()> {
            if self.fail_flush {
                Err(io::Error::other("disk full"))
            } else {
                Ok(())
            }
        }
    }

    fn failing(path: &str, budget: usize, fail_flush: bool) -> Tee {
        Tee::new(path.to_owned(), Box::new(Failing { budget, fail_flush }))
    }

    /// A copy whose write fails stops there; the reader still delivers every
    /// byte, and `finish` names it.
    #[test]
    fn a_tee_whose_write_fails_fails_alone() {
        let mut tee = MultiTeeReader {
            inner: &b"0123456789"[..],
            tees: vec![failing("full.zstd", 4, false)],
        };
        let mut buf = [0u8; 5];
        let mut read = vec![];
        loop {
            let n = tee.read(&mut buf).unwrap();
            if n == 0 {
                break;
            }
            read.extend_from_slice(&buf[..n]);
        }
        assert_eq!(read, b"0123456789");
        let err = tee.finish().unwrap_err().to_string();
        assert!(err.contains("write to 'full.zstd' failed"), "{err}");
    }

    /// A copy whose bytes all went through but whose flush fails is reported.
    #[test]
    fn a_tee_whose_flush_fails_is_reported() {
        let mut tee = MultiTeeReader {
            inner: &b"abc"[..],
            tees: vec![failing("full.zstd", 100, true)],
        };
        std::io::copy(&mut tee, &mut std::io::sink()).unwrap();
        let err = tee.finish().unwrap_err().to_string();
        assert!(err.contains("flush of 'full.zstd' failed"), "{err}");
    }

    /// Every failed copy is named, not only the first.
    #[test]
    fn every_failed_tee_is_reported() {
        let mut tee = MultiTeeReader {
            inner: &b"abc"[..],
            tees: vec![failing("one.zstd", 0, false), failing("two.zstd", 0, false)],
        };
        std::io::copy(&mut tee, &mut std::io::sink()).unwrap();
        let err = tee.finish().unwrap_err().to_string();
        assert!(
            err.contains("one.zstd") && err.contains("two.zstd"),
            "{err}"
        );
    }

    /// A failed compact copy and a failed decoded sink are both named.
    #[test]
    fn join_names_a_failed_copy_and_a_failed_sink() {
        let reader = Err(ExecLogStreamError::IO(io::Error::other(
            "ExecLog: failed to create '/missing/y'",
        )));
        let err = combine(reader, vec!["ExecLog: failed to create '/ro/x'".to_owned()])
            .unwrap_err()
            .to_string();
        assert!(err.contains("/missing/y") && err.contains("/ro/x"), "{err}");
    }

    #[test]
    fn join_keeps_a_clean_or_lone_reader_result() {
        assert_eq!(
            combine(Ok(ReaderOutcome::Read), vec![]).unwrap(),
            ReaderOutcome::Read
        );
        let err = combine(Ok(ReaderOutcome::Read), vec!["sink".to_owned()]).unwrap_err();
        assert!(err.to_string().contains("sink"));
        let decode = prost::DecodeError::new("bad entry");
        let err = combine(Err(ExecLogStreamError::ProstDecode(decode)), vec![]).unwrap_err();
        assert!(matches!(err, ExecLogStreamError::ProstDecode(_)));
        let io_err = ExecLogStreamError::IO(io::Error::other("reader"));
        let err = combine(Err(io_err), vec![]).unwrap_err();
        assert_eq!(err.to_string(), "io error: reader");
    }

    /// A decode error and a failed copy are both reported, without a doubled
    /// `io error:` prefix.
    #[test]
    fn a_reader_error_and_a_failed_copy_are_both_named() {
        let decode = prost::DecodeError::new("bad entry");
        let err = combine(
            Err(ExecLogStreamError::ProstDecode(decode)),
            vec!["ExecLog: write to '/full/c.zstd' failed: disk full".to_owned()],
        )
        .unwrap_err()
        .to_string();
        assert!(
            err.contains("bad entry") && err.contains("/full/c.zstd"),
            "{err}"
        );
        assert_eq!(err.matches("io error:").count(), 1, "{err}");
    }

    /// A log that opens and then fails to read ends the stream with an error
    /// naming it, not as a log read to the end. A directory at the path opens
    /// and then fails every read.
    #[test]
    fn a_log_that_breaks_after_it_appears_fails_the_stream() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("exec.log");
        std::fs::create_dir(&path).unwrap();
        let pid = std::process::id();
        let stream = ExecLogStream::spawn_with_file(path.clone(), pid, pid, vec![]).unwrap();
        let err = stream.join().unwrap_err().to_string();
        assert!(
            err.contains(&format!("reading '{}' failed:", path.display())),
            "{err}"
        );
    }

    /// A break and a failed compact copy are both named, the break first.
    #[test]
    fn a_break_and_a_failed_copy_are_both_named() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("exec.log");
        std::fs::create_dir(&path).unwrap();
        let copy = dir.path().join("missing-dir").join("copy.zstd");
        let pid = std::process::id();
        let stream = ExecLogStream::spawn_with_file(
            path.clone(),
            pid,
            pid,
            vec![copy.to_str().unwrap().to_owned()],
        )
        .unwrap();
        let err = stream.join().unwrap_err().to_string();
        let read_at = err.find(&format!("reading '{}' failed", path.display()));
        let copy_at = err.find(copy.to_str().unwrap());
        assert!(
            matches!((read_at, copy_at), (Some(r), Some(c)) if r < c),
            "{err}"
        );
    }

    /// The error names the path and how many bytes arrived before the break;
    /// any other end is not an error.
    #[test]
    fn broken_read_names_the_path_and_bytes() {
        use crate::stream::Done;
        let path = std::path::Path::new("/ws/exec.log");
        let broken = Done::Broken {
            bytes: 4096,
            error: io::Error::other("Input/output error"),
        };
        let err = broken_read(Some(&broken), path).unwrap_err().to_string();
        assert_eq!(
            err,
            "ExecLog: reading '/ws/exec.log' failed after 4096 bytes: Input/output error"
        );
        let at_start = Done::Broken {
            bytes: 0,
            error: io::Error::other("Is a directory"),
        };
        assert_eq!(
            broken_read(Some(&at_start), path).unwrap_err().to_string(),
            "ExecLog: reading '/ws/exec.log' failed: Is a directory"
        );
        assert!(broken_read(Some(&Done::Drained { bytes: 4096 }), path).is_ok());
        assert!(broken_read(None, path).is_ok());
    }

    /// A log that never appears, with the invocation over, is not an error.
    #[test]
    fn a_log_that_never_appears_is_not_an_error() {
        let gone = {
            let mut child = std::process::Command::new("true").spawn().unwrap();
            let pid = child.id();
            child.wait().unwrap();
            pid
        };
        let dir = tempfile::tempdir().unwrap();
        let stream =
            ExecLogStream::spawn_with_file(dir.path().join("exec.log"), gone, gone, vec![])
                .unwrap();
        assert_eq!(stream.join().unwrap(), ReaderOutcome::NeverAppeared);
    }

    /// A path that cannot be opened at all fails the same way before the log
    /// appears.
    #[test]
    fn a_log_path_that_cannot_open_fails_the_stream() {
        let dir = tempfile::tempdir().unwrap();
        let blocker = dir.path().join("file");
        std::fs::write(&blocker, b"").unwrap();
        let path = blocker.join("exec.log");
        let pid = std::process::id();
        let stream = ExecLogStream::spawn_with_file(path.clone(), pid, pid, vec![]).unwrap();
        let err = stream.join().unwrap_err().to_string();
        assert!(err.contains(path.to_str().unwrap()), "{err}");
    }

    /// `ids` as a compact log: one zstd stream of length-delimited entries.
    fn compact_log(ids: impl IntoIterator<Item = u32>) -> Vec<u8> {
        let raw: Vec<u8> = ids
            .into_iter()
            .flat_map(|id| entry(id).encode_length_delimited_to_vec())
            .collect();
        zstd::encode_all(raw.as_slice(), 0).unwrap()
    }

    /// Bazel starts its log over in place for a retried attempt: both attempts
    /// are decoded, and a copy holds only the last, as Bazel's own file does.
    #[test]
    fn a_log_started_over_in_place_is_decoded_again_and_copies_start_over() {
        let dir = tempfile::tempdir().unwrap();
        let log = dir.path().join("exec.log");
        let copy = dir.path().join("copy.zstd");
        let mut client = std::process::Command::new("sleep")
            .arg("60")
            .spawn()
            .unwrap();
        std::fs::write(&log, compact_log(1..=50)).unwrap();
        let mut stream = ExecLogStream::spawn_with_file(
            log.clone(),
            client.id(),
            client.id(),
            vec![copy.to_str().unwrap().to_owned()],
        )
        .unwrap();
        let recv = stream.receiver().unwrap();
        stream.seal();
        for id in 1..=50 {
            assert_eq!(recv.recv().unwrap().id, id);
        }
        // Shorter than what was read, so the follower sees it truncated.
        let attempt2 = compact_log([100]);
        std::fs::write(&log, &attempt2).unwrap();
        assert_eq!(recv.recv().unwrap().id, 100);
        let _ = client.kill();
        let _ = client.wait();
        assert!(recv.recv().is_err(), "the stream ends with the client");
        assert_eq!(stream.join().unwrap(), ReaderOutcome::Read);
        assert_eq!(std::fs::read(&copy).unwrap(), attempt2);
    }

    /// A sealed stream with no consumer reads the whole log on its own, without
    /// a `join()` to release anything, and hands out no further subscriber.
    #[test]
    fn a_sealed_stream_with_no_consumer_reads_to_the_end_unaided() {
        let dir = tempfile::tempdir().unwrap();
        let log = dir.path().join("exec.log");
        std::fs::write(&log, compact_log(1..=(CHANNEL_CAPACITY as u32 + 10))).unwrap();
        let mut client = std::process::Command::new("true").spawn().unwrap();
        let _ = client.wait();
        let mut stream =
            ExecLogStream::spawn_with_file(log, client.id(), client.id(), vec![]).unwrap();
        stream.seal();
        assert!(
            stream.receiver().is_none(),
            "nothing can subscribe once sealed"
        );

        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(30);
        while !stream.handle.is_finished() {
            assert!(
                std::time::Instant::now() < deadline,
                "a sealed stream with no consumer parked its reader",
            );
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
        assert_eq!(stream.join().unwrap(), ReaderOutcome::Read);
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
