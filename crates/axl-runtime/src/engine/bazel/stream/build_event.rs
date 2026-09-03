use axl_proto::build_event_stream::BuildEvent;
use prost::Message;
use std::fs::File;
use std::io::BufWriter;
use std::io::ErrorKind;
use std::io::Write;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::{env, io};
use std::{
    io::Read,
    path::PathBuf,
    thread::{self, JoinHandle},
};
use thiserror::Error;

use super::broadcaster::{Broadcaster, Subscriber, SubscriberFilter};
use super::redaction::redact_event;
use super::util::{MultiWriter, read_varint};

#[derive(Error, Debug)]
pub enum BuildEventStreamError {
    #[error("io error: {0}")]
    IO(#[from] std::io::Error),
    #[error("prost decode error: {0}")]
    ProstDecode(#[from] prost::DecodeError),
    #[error("prost encode error: {0}")]
    ProstEncode(#[from] prost::EncodeError),
}

/// How often to re-check for a writer on the FIFO — while the watchdog waits
/// for the invocation to end, and in the gap between retry attempts. Short
/// enough to be invisible next to a build, long enough not to spin.
const WRITER_POLL_INTERVAL: std::time::Duration = std::time::Duration::from_millis(10);

/// How many times [`spawn_open_watchdog`] offers to release an open that has
/// not parked yet, before concluding it never will. At
/// [`WRITER_POLL_INTERVAL`] apiece this spans about two seconds, far longer
/// than the microseconds between spawning the watchdog and the open it guards.
const WRITER_RELEASE_OFFERS: u32 = 200;

/// Retries temporary FIFO EOFs below the length-delimited framing.
///
/// A writer may reconnect part-way through a record. The invocation PID bounds
/// the wait because the Bazel server can outlive the command.
struct PendingWriterReader {
    inner: galvanize::Pipe,
    writer_pid: u32,
}

impl Read for PendingWriterReader {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        // `Read` requires an empty buffer to return immediately.
        if buf.is_empty() {
            return Ok(0);
        }
        loop {
            match self.inner.read(buf)? {
                0 => {
                    if !galvanize::is_pid_alive(self.writer_pid) {
                        return Err(io::Error::new(ErrorKind::BrokenPipe, "end of stream"));
                    }
                    thread::sleep(WRITER_POLL_INTERVAL);
                }
                n => return Ok(n),
            }
        }
    }
}

/// Release a reader parked in `Pipe::open` on `path` once `writer_pid` — the
/// invocation expected to write there — is gone.
///
/// Bazel can reject a command line and exit without ever opening the BEP file,
/// leaving its daemon behind to look like a writer still on its way, and a
/// FIFO's blocking open would then park for the life of the process. Guessing
/// wrong is cheap: a FIFO ends only once every writer has closed, so a poke
/// landing beside a writer bazel did open is invisible to the reader.
///
/// The returned flag retires the watchdog; the caller sets it once its own
/// open has returned, so a reader that got its writer honestly is left alone.
fn spawn_open_watchdog(path: PathBuf, writer_pid: u32) -> Arc<AtomicBool> {
    let opened = Arc::new(AtomicBool::new(false));
    let flag = Arc::clone(&opened);
    thread::spawn(move || {
        let mut offers = 0;
        while !flag.load(Ordering::Relaxed) {
            // `poke_writer` only lands while a reader is actually parked, which
            // may still be moments away when the invocation is already over —
            // so keep offering rather than giving up on the first miss.
            if !galvanize::is_pid_alive(writer_pid) {
                if let Ok(true) = galvanize::Pipe::poke_writer(&path) {
                    return;
                }
                offers += 1;
                if offers >= WRITER_RELEASE_OFFERS {
                    return;
                }
            }
            thread::sleep(WRITER_POLL_INTERVAL);
        }
    });
    opened
}

#[derive(Debug)]
pub struct BuildEventStream {
    /// Thread handle, stored in Option so we can take() it to join without consuming self.
    handle: Option<JoinHandle<Result<(), BuildEventStreamError>>>,
    broadcaster: Option<Broadcaster<BuildEvent>>,
}

impl BuildEventStream {
    /// Mint a temp FIFO path and create the inode, returning the path so
    /// the caller can pass it to bazel as `--build_event_binary_file`
    /// before spawning. Pair with `spawn` once the caller has the bazel
    /// child pid in hand.
    pub fn reserve_path() -> io::Result<PathBuf> {
        let out = env::temp_dir().join(format!("build-event-out-{}.bin", uuid::Uuid::new_v4()));
        galvanize::Pipe::mkfifo(&out)?;
        Ok(out)
    }

    /// Spawn the BES reader thread.
    ///
    /// `server_pid` goes to galvanize for its `IfOpenForPid` retry policy
    /// (it's the pid that *holds the FIFO write end open during an
    /// attempt*; for real bazel that's the server/daemon pid).
    ///
    /// `writer_pid` is the per-invocation pid whose death means no more bytes
    /// are coming — for real bazel the spawned client process
    /// (`Command::spawn().id()`). It answers both "will a writer ever arrive?"
    /// (see [`spawn_open_watchdog`]) and "is another attempt coming?" (the
    /// `BrokenPipe` branch in the read loop). Neither question can be put to
    /// `server_pid`, since the daemon outlives the invocation and so always
    /// looks like a writer still on its way.
    ///
    /// Each `(path, signal)` pair gets raw FIFO bytes mirrored to `path`;
    /// `signal.complete(result)` fires after flush, unblocking the file
    /// sink handle's `wait()`.
    pub fn spawn(
        path: PathBuf,
        server_pid: u32,
        writer_pid: u32,
        file_sinks: Vec<(String, std::sync::Arc<super::super::build::FileSignal>)>,
    ) -> io::Result<Self> {
        let main_broadcaster = Broadcaster::new();
        let thread_broadcaster = main_broadcaster.clone();
        let handle = thread::spawn(move || {
            let broadcaster = thread_broadcaster;
            let raw_file_sink_paths: Vec<String> =
                file_sinks.iter().map(|(p, _)| p.clone()).collect();
            let file_signals: Vec<std::sync::Arc<super::super::build::FileSignal>> =
                file_sinks.into_iter().map(|(_, s)| s).collect();
            let signal_all = |result: Result<(), String>| {
                for s in &file_signals {
                    s.complete(result.clone());
                }
            };

            let open_file_sinks = |paths: &[String]| -> io::Result<MultiWriter<BufWriter<File>>> {
                let writers = paths
                    .iter()
                    .map(|p| Ok(BufWriter::new(File::create(p)?)))
                    .collect::<io::Result<Vec<_>>>()?;
                Ok(MultiWriter { writers })
            };

            // End the stream: cut subscribers loose, flush and signal the
            // file sinks. Says nothing about whether the build succeeded —
            // that is the caller's to read off bazel's exit code.
            let finish = |raw_out: &mut MultiWriter<BufWriter<File>>| {
                broadcaster.close();
                match raw_out.flush() {
                    Ok(()) => {
                        signal_all(Ok(()));
                        Ok(())
                    }
                    Err(e) => {
                        signal_all(Err(format!("flush failed: {e}")));
                        Err(BuildEventStreamError::IO(e))
                    }
                }
            };

            let mut raw_out = match open_file_sinks(&raw_file_sink_paths) {
                Ok(w) => w,
                Err(e) => {
                    signal_all(Err(format!("failed to open file sink(s): {e}")));
                    return Err(BuildEventStreamError::IO(e));
                }
            };
            // mkfifo is idempotent (tolerates EEXIST), so this works whether
            // the caller pre-created the FIFO via `reserve_path` (production)
            // or not (unit tests). Before the watchdog, which has nothing to
            // poke until the inode exists.
            galvanize::Pipe::mkfifo(&path)?;

            let opened = spawn_open_watchdog(path.clone(), writer_pid);
            let mut reader = PendingWriterReader {
                inner: galvanize::Pipe::open(
                    path,
                    galvanize::RetryPolicy::IfOpenForPid(server_pid),
                )?,
                writer_pid,
            };
            opened.store(true, Ordering::Relaxed);

            let mut buf: Vec<u8> = Vec::with_capacity(1024 * 5);
            // Initial size for reading a varint
            buf.resize(10, 0);

            // Scratch buffer for re-encoding redacted events. Reused across
            // iterations to avoid per-event allocation. Only populated on the
            // rare modified-event path (the common case writes the original
            // bytes straight through).
            let mut reencode_buf: Vec<u8> = Vec::with_capacity(1024);

            let read_event = |buf: &mut Vec<u8>,
                              reencode_buf: &mut Vec<u8>,
                              raw_out: &mut MultiWriter<BufWriter<File>>,
                              reader: &mut PendingWriterReader|
             -> Result<BuildEvent, BuildEventStreamError> {
                let (size, vbuf) = read_varint(reader)?;
                if size > buf.len() {
                    buf.resize(size, 0);
                }
                reader.read_exact(&mut buf[0..size])?;
                let mut event = BuildEvent::decode(&buf[0..size])?;
                // Redact secrets (headers, env passthrough, URL creds) BEFORE
                // anything downstream sees the event. Raw file sinks, gRPC
                // forwarders, and AXL iterators all read from the post-redact
                // stream — secrets never leave this process.
                //
                // `redact_event` only mutates a small set of payload kinds
                // (StructuredCommandLine, UnstructuredCommandLine, BuildMetadata,
                // etc.); for the common case it returns false and we write the
                // original bytes straight through with no re-encode cost.
                let modified = redact_event(&mut event);
                // These can be extremely slow and expensive calls depending
                // on the destination that we are writing to.
                // TODO: Ensure we have a dedicated thread where the writing
                // happens to avoid stalling.
                if modified {
                    reencode_buf.clear();
                    event.encode_length_delimited(reencode_buf)?;
                    raw_out.write_all(reencode_buf.as_slice())?;
                } else {
                    raw_out.write(vbuf.as_slice())?;
                    raw_out.write_all(&buf[0..size])?;
                }
                Ok(event)
            };

            // Set when BuildFinished arrives with REMOTE_CACHE_EVICTED (code 39).
            // While true, a BrokenPipe (attempt N's writer closing) is swallowed
            // so the FIFO read end stays open for the retry writer to reconnect.
            // Cleared when the next BuildStarted arrives, meaning the retry has
            // connected and normal EOF handling resumes.
            let mut expecting_retry = false;

            loop {
                match read_event(&mut buf, &mut reencode_buf, &mut raw_out, &mut reader) {
                    Ok(event) => {
                        let last_message = event.last_message;

                        use axl_proto::build_event_stream::build_event::Payload;
                        match &event.payload {
                            Some(Payload::Finished(finished)) => {
                                if finished
                                    .exit_code
                                    .as_ref()
                                    .map(|c| c.code == 39)
                                    .unwrap_or(false)
                                {
                                    expecting_retry = true;
                                    // Replace file sinks with fresh truncated files so
                                    // the retry stream starts clean.
                                    //
                                    // Order matters: flush first to empty the BufWriter's
                                    // internal buffer, then open new sinks (File::create
                                    // truncates the file).  The assignment drops the old
                                    // MultiWriter; since its BufWriters are now empty,
                                    // the drop-flush is a no-op and no stale bytes are
                                    // written after the truncation.
                                    let _ = raw_out.flush();
                                    match open_file_sinks(&raw_file_sink_paths) {
                                        Ok(new_raw_out) => {
                                            raw_out = new_raw_out;
                                        }
                                        Err(e) => {
                                            signal_all(Err(format!(
                                                "failed to reopen file sink(s) for retry: {e}"
                                            )));
                                            broadcaster.close();
                                            return Err(BuildEventStreamError::IO(e));
                                        }
                                    }
                                }
                            }
                            Some(Payload::Started(_)) => {
                                expecting_retry = false;
                            }
                            _ => {}
                        }

                        // Fan-out to all subscribers (non-blocking)
                        broadcaster.send(event);

                        if last_message && !expecting_retry {
                            return finish(&mut raw_out);
                        }
                    }
                    Err(BuildEventStreamError::IO(err)) if err.kind() == ErrorKind::BrokenPipe => {
                        if expecting_retry {
                            // aspect-build/aspect-cli#1060: a REMOTE_CACHE_EVICTED
                            // BuildFinished is not always followed by a retry.
                            // Bazel may emit it as the last message and then
                            // the invocation ends. If the writer process is
                            // gone, no retry is coming — close gracefully
                            // instead of spinning.
                            //
                            // Pid liveness rather than `is_path_open_for_pid`:
                            // Bazel closes the BEP file at the end of every
                            // attempt and reopens it for the next
                            // (FileTransport.SequentialWriter.close()), so
                            // during that gap the writer is alive with no
                            // writer attached to the FIFO. Reading that as "no
                            // retry coming" would drop attempt 2's events.
                            if !galvanize::is_pid_alive(writer_pid) {
                                return finish(&mut raw_out);
                            }
                            // Writer is alive; Bazel is between attempts.
                            // With no writer attached, read() on the FIFO
                            // returns 0 immediately, so looping without
                            // backoff creates a hot CPU spin until the next
                            // writer opens the pipe. Sleep briefly to yield
                            // the CPU between polls.
                            std::thread::sleep(WRITER_POLL_INTERVAL);
                            continue;
                        }
                        return finish(&mut raw_out);
                    }
                    Err(err) => {
                        signal_all(Err(format!("BES read error: {err}")));
                        broadcaster.close();
                        return Err(err);
                    }
                }
            }
        });

        Ok(Self {
            handle: Some(handle),
            broadcaster: Some(main_broadcaster),
        })
    }

    /// Subscribe to the build event stream without history replay.
    ///
    /// This is for internal use by sinks that subscribe at stream creation time
    /// and don't need history replay. Use `subscribe()` for user-facing APIs
    /// where late subscription support is needed.
    pub fn subscribe(&self) -> Subscriber<BuildEvent> {
        self.subscribe_filtered(None)
    }

    /// Subscribe with an optional send-side filter (see
    /// [`Broadcaster::subscribe_filtered`]). A filtered subscriber's buffer
    /// only holds events the filter accepts — the reader thread skips the rest
    /// before cloning them, so a `kinds=`-scoped AXL iterator never pays for
    /// the event kinds it doesn't consume.
    pub fn subscribe_filtered(
        &self,
        filter: Option<SubscriberFilter<BuildEvent>>,
    ) -> Subscriber<BuildEvent> {
        match self.broadcaster.as_ref() {
            Some(b) => b.subscribe_filtered(filter),
            // Stream has already been joined.
            None => Subscriber::disconnected(),
        }
    }

    /// Wait for the BES thread to complete.
    pub fn join(&mut self) -> Result<(), BuildEventStreamError> {
        let _ = self.broadcaster.take();
        if let Some(handle) = self.handle.take() {
            handle.join().expect("join error")?;
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs::OpenOptions;

    use std::time::Duration;

    /// Encode a `BuildEvent` as a length-delimited protobuf record (LEB128 varint + body),
    /// matching the format that `read_varint` + `read_exact` expects on the read side.
    fn encode_event(event: &BuildEvent) -> Vec<u8> {
        let body = event.encode_to_vec();
        let mut out = Vec::new();
        let mut remaining = body.len();
        loop {
            let mut byte = (remaining & 0x7F) as u8;
            remaining >>= 7;
            if remaining > 0 {
                byte |= 0x80;
            }
            out.push(byte);
            if remaining == 0 {
                break;
            }
        }
        out.extend_from_slice(&body);
        out
    }

    fn make_event(last_message: bool) -> BuildEvent {
        BuildEvent {
            last_message,
            ..Default::default()
        }
    }

    fn temp_fifo_path() -> PathBuf {
        std::env::temp_dir().join(format!("test-bes-{}.fifo", uuid::Uuid::new_v4()))
    }

    /// Poll until the FIFO inode appears at `path`. The BES thread mkfifos the
    /// path lazily inside `BuildEventStream::spawn`, so test threads that open
    /// the writer side have to wait for the inode before `OpenOptions::open`.
    fn wait_for_fifo(path: &PathBuf) {
        let deadline = std::time::Instant::now() + Duration::from_secs(5);
        while !path.exists() {
            assert!(
                std::time::Instant::now() < deadline,
                "FIFO was not created within 5s: {}",
                path.display()
            );
            std::thread::sleep(Duration::from_millis(5));
        }
    }

    /// Spawn a long-lived `sleep` subprocess and return its pid.
    ///
    /// We need a live, external pid for `RetryPolicy::IfOpenForPid` that does NOT
    /// hold the FIFO open.  Using our own pid would cause `is_path_open_for_pid`
    /// to return `true` (we own the read end), preventing BrokenPipe from firing.
    fn spawn_pid_holder() -> std::process::Child {
        std::process::Command::new("sleep")
            .arg("60")
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .spawn()
            .expect("failed to spawn sleep")
    }

    /// A pid that is certainly gone: spawn the shortest-lived process there is
    /// and reap it, so `is_pid_alive` sees a free pid rather than a zombie.
    fn dead_pid() -> u32 {
        let mut child = std::process::Command::new("true")
            .spawn()
            .expect("failed to spawn true");
        let pid = child.id();
        child.wait().expect("failed to reap true");
        pid
    }

    // -------------------------------------------------------------------------
    // No writer ever arrives: Bazel exited before opening the BEP file
    // -------------------------------------------------------------------------

    /// The watchdog usually starts while the invocation is still alive, but
    /// nothing guarantees it: an invocation already over when the watchdog
    /// starts means the first poke arrives before there is any reader to
    /// release. Poking is only effective against a parked reader, so the
    /// watchdog has to keep offering — one attempt would strand this reader.
    #[test]
    fn the_watchdog_waits_for_a_reader_that_has_not_parked_yet() {
        let released = crate::test::with_timeout(Duration::from_secs(10), || {
            let path = temp_fifo_path();
            galvanize::Pipe::mkfifo(&path).expect("mkfifo");

            // Dead from the outset, so the watchdog starts poking immediately —
            // well before the open below exists to be released.
            let opened = spawn_open_watchdog(path.clone(), dead_pid());
            std::thread::sleep(Duration::from_millis(100));

            let reader = galvanize::Pipe::open(path, galvanize::RetryPolicy::Never);
            opened.store(true, Ordering::Relaxed);
            reader.is_ok()
        });
        assert_eq!(
            released,
            Some(true),
            "the watchdog must keep offering until the reader is there to release"
        );
    }

    /// Bazel can exit before it ever opens the BEP file — an unrecognized flag,
    /// a bad startup option — leaving a live daemon that still looks like a
    /// writer on its way. The stream has to end, empty, rather than wait on a
    /// writer that cannot arrive; the caller reports bazel's exit code.
    #[test]
    fn a_writer_that_never_opens_ends_the_stream() {
        let outcome = crate::test::with_timeout(Duration::from_secs(10), || {
            let path = temp_fifo_path();
            // Server alive (daemons outlive the invocation), invocation gone.
            let mut holder = spawn_pid_holder();
            let mut stream =
                BuildEventStream::spawn(path, holder.id(), dead_pid(), vec![]).unwrap();
            let sub = stream.subscribe();
            let joined = stream.join().is_ok();
            let events: Vec<_> = std::iter::from_fn(|| sub.recv().ok()).collect();
            let _ = holder.kill();
            (joined, events.len())
        });
        assert_eq!(
            outcome,
            Some((true, 0)),
            "the stream must end promptly and empty, not wait for a writer that cannot come"
        );
    }

    /// The other edge: the reader starts the moment bazel is spawned, but bazel
    /// does not open the BEP file until its JVM is up seconds later. That gap
    /// must not be taken for an invocation that will never write, or every
    /// event of a real build is lost.
    #[test]
    fn a_writer_that_opens_late_still_delivers_its_events() {
        let path = temp_fifo_path();
        let mut holder = spawn_pid_holder();
        let pid = holder.id();

        let mut stream = BuildEventStream::spawn(path.clone(), pid, pid, vec![]).unwrap();
        let sub = stream.subscribe();
        wait_for_fifo(&path);

        let path_w = path.clone();
        let writer = std::thread::spawn(move || {
            // Many poll intervals' worth of "no writer", so the reader cannot
            // pass this test by racing to its first read.
            std::thread::sleep(Duration::from_millis(300));
            let mut f = OpenOptions::new().write(true).open(&path_w).unwrap();
            f.write_all(&encode_event(&make_event(true))).unwrap();
        });

        writer.join().unwrap();
        stream.join().unwrap();
        let _ = holder.kill();

        let events: Vec<_> = std::iter::from_fn(|| sub.recv().ok()).collect();
        assert_eq!(
            events.len(),
            1,
            "a late writer's events must not be dropped"
        );
        assert!(events[0].last_message);
    }

    // -------------------------------------------------------------------------
    // Happy path: Bazel sends a complete stream ending with last_message = true
    // -------------------------------------------------------------------------

    #[test]
    fn test_complete_stream_delivers_all_events() {
        let path = temp_fifo_path();
        let mut holder = spawn_pid_holder();
        let pid = holder.id();

        let mut stream = BuildEventStream::spawn(path.clone(), pid, pid, vec![]).unwrap();
        let sub = stream.subscribe();
        wait_for_fifo(&path);

        let path_w = path.clone();
        let writer = std::thread::spawn(move || {
            let mut f = OpenOptions::new().write(true).open(&path_w).unwrap();
            f.write_all(&encode_event(&make_event(false))).unwrap();
            f.write_all(&encode_event(&make_event(false))).unwrap();
            f.write_all(&encode_event(&make_event(true))).unwrap();
        });

        writer.join().unwrap();
        stream.join().unwrap();
        let _ = holder.kill();

        let events: Vec<_> = std::iter::from_fn(|| sub.recv().ok()).collect();
        assert_eq!(events.len(), 3);
        assert!(!events[0].last_message);
        assert!(!events[1].last_message);
        assert!(events[2].last_message);
    }

    // -------------------------------------------------------------------------
    // BrokenPipe: Bazel closes the FIFO before sending last_message
    // (e.g. Bazel process killed, SIGTERM, or a transient-error attempt ending)
    // -------------------------------------------------------------------------

    #[test]
    fn test_broken_pipe_ends_stream_gracefully() {
        let path = temp_fifo_path();
        let mut holder = spawn_pid_holder();
        let pid = holder.id();

        let mut stream = BuildEventStream::spawn(path.clone(), pid, pid, vec![]).unwrap();
        let sub = stream.subscribe();
        wait_for_fifo(&path);

        let path_w = path.clone();
        let writer = std::thread::spawn(move || {
            let mut f = OpenOptions::new().write(true).open(&path_w).unwrap();
            f.write_all(&encode_event(&make_event(false))).unwrap();
            // Closing `f` here without last_message triggers BrokenPipe on the read side.
        });

        writer.join().unwrap();
        // join() must return Ok — BrokenPipe is a graceful termination.
        stream.join().unwrap();
        let _ = holder.kill();

        let events: Vec<_> = std::iter::from_fn(|| sub.recv().ok()).collect();
        assert_eq!(events.len(), 1);
        assert!(!events[0].last_message);
    }

    // -------------------------------------------------------------------------
    // Writer reconnect
    //
    // Force reconnect gaps to surface as `Ok(0)`: this process owns the FIFO's
    // read end, while a separate live PID represents the Bazel invocation.
    // -------------------------------------------------------------------------

    /// Keeps the stream open when a writer reconnects between records.
    #[test]
    fn a_writer_reconnecting_between_records_keeps_the_stream_open() {
        let outcome = crate::test::with_timeout(Duration::from_secs(10), || {
            let path = temp_fifo_path();
            let mut holder = spawn_pid_holder();

            let mut stream =
                BuildEventStream::spawn(path.clone(), std::process::id(), holder.id(), vec![])
                    .unwrap();
            let sub = stream.subscribe();
            wait_for_fifo(&path);

            let path_w = path.clone();
            let _writer = std::thread::spawn(move || {
                let mut f = OpenOptions::new().write(true).open(&path_w).unwrap();
                f.write_all(&encode_event(&make_event(false))).unwrap();
                drop(f);
                // Keep the writer disconnected for several polling intervals.
                std::thread::sleep(Duration::from_millis(200));
                let mut f = OpenOptions::new().write(true).open(&path_w).unwrap();
                f.write_all(&encode_event(&make_event(true))).unwrap();
            });

            // Do not join: if the reader exits, the reconnect blocks in `open`,
            // and the outer timeout must report the failure.
            let joined = stream.join().map_err(|e| e.to_string());
            let events: Vec<_> = std::iter::from_fn(|| sub.recv().ok()).collect();
            let _ = holder.kill();
            (joined, events.len())
        });
        assert_eq!(
            outcome,
            Some((Ok(()), 2)),
            "the gap between attempts must not end the stream"
        );
    }

    /// Preserves framing when a writer reconnects part-way through a record.
    #[test]
    fn a_record_split_across_a_writer_reconnect_is_not_desynced() {
        let outcome = crate::test::with_timeout(Duration::from_secs(10), || {
            let path = temp_fifo_path();
            let mut holder = spawn_pid_holder();

            let mut stream =
                BuildEventStream::spawn(path.clone(), std::process::id(), holder.id(), vec![])
                    .unwrap();
            let sub = stream.subscribe();
            wait_for_fifo(&path);

            let path_w = path.clone();
            let _writer = std::thread::spawn(move || {
                let record = encode_event(&make_event(true));
                // Reconnect before the final byte so the gap occurs inside the record.
                let split = record.len() - 1;
                let mut f = OpenOptions::new().write(true).open(&path_w).unwrap();
                f.write_all(&record[..split]).unwrap();
                drop(f);
                std::thread::sleep(Duration::from_millis(200));
                let mut f = OpenOptions::new().write(true).open(&path_w).unwrap();
                f.write_all(&record[split..]).unwrap();
            });

            // Do not join: if the reader exits, the reconnect blocks in `open`,
            // and the outer timeout must report the failure.
            let joined = stream.join().map_err(|e| e.to_string());
            let events: Vec<_> = std::iter::from_fn(|| sub.recv().ok()).collect();
            let _ = holder.kill();
            (joined, events.len(), events.first().map(|e| e.last_message))
        });
        assert_eq!(
            outcome,
            Some((Ok(()), 1, Some(true))),
            "a record spanning a reconnect must be delivered whole"
        );
    }

    // -------------------------------------------------------------------------
    // File sink: raw bytes are written to a file sink path alongside events
    // -------------------------------------------------------------------------

    #[test]
    fn test_file_sink_captures_raw_bytes() {
        let path = temp_fifo_path();
        let sink_path =
            std::env::temp_dir().join(format!("test-bes-sink-{}.bin", uuid::Uuid::new_v4()));
        let mut holder = spawn_pid_holder();
        let pid = holder.id();

        let mut stream = BuildEventStream::spawn(
            path.clone(),
            pid,
            pid,
            vec![(
                sink_path.to_str().unwrap().to_string(),
                std::sync::Arc::new(crate::engine::bazel::build::FileSignal::new()),
            )],
        )
        .unwrap();
        wait_for_fifo(&path);

        let events_to_send = vec![make_event(false), make_event(true)];
        let raw_bytes: Vec<u8> = events_to_send.iter().flat_map(encode_event).collect();

        let path_w = path.clone();
        let raw_clone = raw_bytes.clone();
        let writer = std::thread::spawn(move || {
            let mut f = OpenOptions::new().write(true).open(&path_w).unwrap();
            f.write_all(&raw_clone).unwrap();
        });

        writer.join().unwrap();
        stream.join().unwrap();
        let _ = holder.kill();

        let written = std::fs::read(&sink_path).unwrap();
        assert_eq!(
            written, raw_bytes,
            "sink file must contain the exact raw bytes"
        );
        let _ = std::fs::remove_file(&sink_path);
    }

    // -------------------------------------------------------------------------
    // Transient error retry (documents current behavior)
    //
    // When Bazel retries after REMOTE_CACHE_EVICTED it reopens the same BEP
    // path.  With our FIFO-based design the sequence is:
    //
    //   Attempt 1: Bazel opens FIFO → writes → closes
    //              → our thread sees BrokenPipe → closes broadcaster → exits
    //   Attempt 2: Bazel tries to open FIFO again for writing
    //              → BLOCKS: the read end was closed when our thread exited,
    //                so there is no reader; the open(O_WRONLY) never returns.
    //
    // Net effect: only attempt 1's events are visible to subscribers.
    // Attempt 2 cannot deliver any events through the closed stream.
    // -------------------------------------------------------------------------

    #[test]
    fn test_transient_retry_cannot_reconnect_after_stream_closed() {
        let path = temp_fifo_path();
        let mut holder = spawn_pid_holder();
        let pid = holder.id();

        let mut stream = BuildEventStream::spawn(path.clone(), pid, pid, vec![]).unwrap();
        let sub = stream.subscribe();
        wait_for_fifo(&path);

        // Attempt 1 — writer closes without last_message (transient error).
        {
            let mut f = OpenOptions::new().write(true).open(&path).unwrap();
            f.write_all(&encode_event(&make_event(false))).unwrap();
            // `f` drops here → all writers gone → BrokenPipe in stream thread.
        }

        stream.join().unwrap();
        let _ = holder.kill();

        // Only attempt 1's single event was received.
        let events: Vec<_> = std::iter::from_fn(|| sub.recv().ok()).collect();
        assert_eq!(events.len(), 1, "only attempt 1 events should be visible");

        // Attempt 2 — try to open the write end with O_NONBLOCK.
        // Because the stream thread exited and closed the read end, POSIX
        // requires O_WRONLY|O_NONBLOCK on a FIFO with no reader to return
        // ENXIO immediately.  This confirms the read end is gone without
        // leaving a dangling blocked thread.
        use nix::libc;
        use std::os::unix::fs::OpenOptionsExt;
        let result = OpenOptions::new()
            .write(true)
            .custom_flags(libc::O_NONBLOCK)
            .open(&path);
        assert!(result.is_err(), "attempt 2 open should fail (no reader)");
        assert_eq!(
            result.unwrap_err().raw_os_error(),
            Some(libc::ENXIO),
            "expected ENXIO (no reader on FIFO)"
        );
    }

    // -------------------------------------------------------------------------
    // Desired behavior: transient error retry delivers events from both attempts
    //
    // Bazel REMOTE_CACHE_EVICTED retry sequence (from transient_error.md):
    //   Attempt 1: open FIFO → write BuildStarted + BuildFinished(FAILED) → close
    //   Attempt 2: open FIFO → write BuildStarted + BuildFinished(SUCCESS) → close
    //
    // The subscriber should see all 4 events in order across both attempts.
    //
    // NOT YET IMPLEMENTED: the stream thread currently exits on the first
    // BrokenPipe (attempt 1 close), closing the read end.  Attempt 2's
    // open(O_WRONLY) then blocks indefinitely waiting for a reader.
    //
    // To make this pass the stream would need to keep the FIFO read end open
    // (or reopen it) after BrokenPipe so that the retry writer can connect.
    // -------------------------------------------------------------------------

    fn make_build_started() -> BuildEvent {
        use axl_proto::build_event_stream::BuildStarted;
        use axl_proto::build_event_stream::build_event::Payload;
        BuildEvent {
            last_message: false,
            payload: Some(Payload::Started(BuildStarted::default())),
            ..Default::default()
        }
    }

    fn make_build_finished(exit_code: i32, last_message: bool) -> BuildEvent {
        use axl_proto::build_event_stream::BuildFinished;
        use axl_proto::build_event_stream::build_event::Payload;
        use axl_proto::build_event_stream::build_finished::ExitCode;
        BuildEvent {
            last_message,
            payload: Some(Payload::Finished(BuildFinished {
                exit_code: Some(ExitCode {
                    code: exit_code,
                    ..Default::default()
                }),
                ..Default::default()
            })),
            ..Default::default()
        }
    }

    #[test]
    fn test_transient_retry_delivers_events_from_both_attempts() {
        let path = temp_fifo_path();
        let mut holder = spawn_pid_holder();
        let pid = holder.id();

        let mut stream = BuildEventStream::spawn(path.clone(), pid, pid, vec![]).unwrap();
        let sub = stream.subscribe();
        wait_for_fifo(&path);

        // Attempt 1: BuildStarted + BuildFinished(REMOTE_CACHE_EVICTED=39), then
        // the writer closes.  The stream should swallow the BrokenPipe and wait.
        {
            let mut f = OpenOptions::new().write(true).open(&path).unwrap();
            f.write_all(&encode_event(&make_build_started())).unwrap();
            f.write_all(&encode_event(&make_build_finished(39, false)))
                .unwrap();
        }

        // Attempt 2: Bazel retries.  Because the stream kept the FIFO read end
        // open, this open(O_WRONLY) pairs with it immediately.
        {
            let mut f = OpenOptions::new().write(true).open(&path).unwrap();
            f.write_all(&encode_event(&make_build_started())).unwrap();
            f.write_all(&encode_event(&make_build_finished(0, true)))
                .unwrap();
        }

        stream.join().unwrap();
        let _ = holder.kill();

        let events: Vec<_> = std::iter::from_fn(|| sub.recv().ok()).collect();

        assert_eq!(events.len(), 4);
        // attempt 1
        assert!(matches!(
            events[0].payload,
            Some(axl_proto::build_event_stream::build_event::Payload::Started(_))
        ));
        assert!(matches!(
            events[1].payload,
            Some(axl_proto::build_event_stream::build_event::Payload::Finished(_))
        ));
        // attempt 2
        assert!(matches!(
            events[2].payload,
            Some(axl_proto::build_event_stream::build_event::Payload::Started(_))
        ));
        assert!(events[3].last_message);
    }

    /// Bazel may set last_message=true on the BuildFinished(REMOTE_CACHE_EVICTED)
    /// event.  The stream must not close on that last_message because a retry is
    /// expected; it should stay open and deliver attempt 2's events normally.
    #[test]
    fn test_transient_retry_with_last_message_on_evicted_event() {
        let path = temp_fifo_path();
        let mut holder = spawn_pid_holder();
        let pid = holder.id();

        let mut stream = BuildEventStream::spawn(path.clone(), pid, pid, vec![]).unwrap();
        let sub = stream.subscribe();
        wait_for_fifo(&path);

        // Attempt 1: BuildFinished carries both REMOTE_CACHE_EVICTED and last_message=true.
        {
            let mut f = OpenOptions::new().write(true).open(&path).unwrap();
            f.write_all(&encode_event(&make_build_started())).unwrap();
            f.write_all(&encode_event(&make_build_finished(39, true)))
                .unwrap();
        }

        // Attempt 2: normal successful stream.
        {
            let mut f = OpenOptions::new().write(true).open(&path).unwrap();
            f.write_all(&encode_event(&make_build_started())).unwrap();
            f.write_all(&encode_event(&make_build_finished(0, true)))
                .unwrap();
        }

        stream.join().unwrap();
        let _ = holder.kill();

        let events: Vec<_> = std::iter::from_fn(|| sub.recv().ok()).collect();
        // Receiving all 4 events proves the stream did not terminate when it saw
        // last_message=true on attempt 1's REMOTE_CACHE_EVICTED BuildFinished.
        assert_eq!(events.len(), 4);
        assert!(events[1].last_message); // attempt 1's BuildFinished had last_message=true ...
        assert!(events[3].last_message); // ... but only attempt 2's actually closed the stream
    }

    // -------------------------------------------------------------------------
    // File sink truncation on retry
    // -------------------------------------------------------------------------

    fn attempt1_raw(last_message: bool) -> Vec<u8> {
        [
            encode_event(&make_build_started()),
            encode_event(&make_build_finished(39, last_message)),
        ]
        .concat()
    }

    fn attempt2_raw() -> Vec<u8> {
        [
            encode_event(&make_build_started()),
            encode_event(&make_build_finished(0, true)),
        ]
        .concat()
    }

    fn run_retry_sink_test(last_message_on_evicted: bool) -> Vec<u8> {
        let path = temp_fifo_path();
        let sink_path =
            std::env::temp_dir().join(format!("test-bes-sink-{}.bin", uuid::Uuid::new_v4()));
        let mut holder = spawn_pid_holder();
        let pid = holder.id();

        let mut stream = BuildEventStream::spawn(
            path.clone(),
            pid,
            pid,
            vec![(
                sink_path.to_str().unwrap().to_string(),
                std::sync::Arc::new(crate::engine::bazel::build::FileSignal::new()),
            )],
        )
        .unwrap();
        wait_for_fifo(&path);

        {
            let mut f = OpenOptions::new().write(true).open(&path).unwrap();
            f.write_all(&attempt1_raw(last_message_on_evicted)).unwrap();
        }
        {
            let mut f = OpenOptions::new().write(true).open(&path).unwrap();
            f.write_all(&attempt2_raw()).unwrap();
        }

        stream.join().unwrap();
        let _ = holder.kill();

        let written = std::fs::read(&sink_path).unwrap();
        let _ = std::fs::remove_file(&sink_path);
        written
    }

    /// After REMOTE_CACHE_EVICTED (last_message=false), the file sinks must be
    /// truncated so they contain only the retry stream's bytes.
    #[test]
    fn test_file_sinks_emptied_before_retry_stream() {
        let written = run_retry_sink_test(false);
        assert_eq!(written, attempt2_raw());
    }

    /// Same requirement when Bazel sets last_message=true on the evicted event.
    #[test]
    fn test_file_sinks_emptied_before_retry_stream_last_message() {
        let written = run_retry_sink_test(true);
        assert_eq!(written, attempt2_raw());
    }
}
