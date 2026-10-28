use prost::Message;
use prost::bytes::Bytes;
use std::fs::File;
use std::io;
use std::io::BufWriter;
use std::io::Write;
use std::sync::{Arc, Condvar, Mutex, OnceLock};
use std::thread::{self, JoinHandle};
use thiserror::Error;

use super::BuildEventEnvelope;
use super::binary_file::{End, Frame, Reader};
use super::redact::redact_event;
use crate::fanout::{Broadcaster, Subscriber, SubscriberFilter};
use crate::stream::Gate;
use crate::stream::pipe::Channel;

/// Set once, when the feed has flushed the raw file a file sink mirrors the
/// stream to, so whoever waits on that sink learns whether the copy is whole.
pub struct FileSignal {
    result: Mutex<Option<Result<(), String>>>,
    cv: Condvar,
}

impl FileSignal {
    pub fn new() -> Self {
        Self {
            result: Mutex::new(None),
            cv: Condvar::new(),
        }
    }

    pub fn complete(&self, result: Result<(), String>) {
        let mut guard = self.result.lock().unwrap();
        if guard.is_none() {
            *guard = Some(result);
            self.cv.notify_all();
        }
    }

    pub fn is_complete(&self) -> bool {
        self.result.lock().unwrap().is_some()
    }

    pub fn wait(&self) -> Result<(), String> {
        let mut guard = self.result.lock().unwrap();
        while guard.is_none() {
            guard = self.cv.wait(guard).unwrap();
        }
        guard.as_ref().unwrap().clone()
    }
}

struct MultiWriter<R: Write> {
    writers: Vec<R>,
}

impl<R> io::Write for MultiWriter<R>
where
    R: io::Write,
{
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        for w in &mut self.writers {
            w.write_all(buf)?;
        }
        Ok(buf.len())
    }
    fn flush(&mut self) -> io::Result<()> {
        for w in &mut self.writers {
            w.flush()?;
        }
        Ok(())
    }
}

#[derive(Error, Debug)]
pub enum BuildEventStreamError {
    #[error("io error: {0}")]
    IO(#[from] std::io::Error),
    #[error("prost decode error: {0}")]
    ProstDecode(#[from] prost::DecodeError),
    #[error("prost encode error: {0}")]
    ProstEncode(#[from] prost::EncodeError),
}

#[derive(Debug)]
pub struct BuildEventStream {
    /// Thread handle, stored in Option so we can take() it to join without consuming self.
    handle: Option<JoinHandle<Result<(), BuildEventStreamError>>>,
    broadcaster: Option<Broadcaster<Arc<BuildEventEnvelope>>>,
    /// How the stream ended, set by the reader thread before it closes the
    /// broadcaster, so anything woken by the close can read it.
    end: Arc<OnceLock<End>>,
}

impl BuildEventStream {
    /// Reserve the FIFO the BEP is written to, so its path can go on the
    /// command line as `--build_event_binary_file` before bazel is spawned.
    /// Pair with [`spawn`](Self::spawn) once the bazel client exists.
    pub fn reserve() -> io::Result<Channel> {
        Channel::fifo("build-event-out")
    }

    /// Spawn the BES reader thread on `channel`.
    ///
    /// `gate` answers the two questions that decide when the stream is over
    /// (see [`crate::stream`]); for a real bazel it comes from
    /// `PidPair::gate`, which keys "ended" on the client and "held" on the
    /// server.
    ///
    /// Each `(path, signal)` pair gets raw FIFO bytes mirrored to `path`;
    /// `signal.complete(result)` fires after flush, unblocking the file
    /// sink handle's `wait()`.
    pub fn spawn(
        channel: Channel,
        gate: Gate,
        file_sinks: Vec<(String, Arc<FileSignal>)>,
    ) -> io::Result<Self> {
        let main_broadcaster = Broadcaster::new();
        let thread_broadcaster = main_broadcaster.clone();
        let end_cell = Arc::new(OnceLock::new());
        let thread_end = end_cell.clone();
        // Opened here, not on the thread: it never blocks, and a failure
        // belongs to the caller.
        let pipe = channel.follow(gate)?;
        let handle = thread::spawn(move || {
            let broadcaster = thread_broadcaster;
            let raw_file_sink_paths: Vec<String> =
                file_sinks.iter().map(|(p, _)| p.clone()).collect();
            let file_signals: Vec<Arc<FileSignal>> =
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
            // Scratch buffer for re-encoding redacted events. Reused across
            // iterations to avoid per-event allocation. Only populated on the
            // rare modified-event path (the common case writes the original
            // bytes straight through).
            let mut reencode_buf: Vec<u8> = Vec::with_capacity(1024);

            // One event, from the wire to what subscribers and file sinks get.
            // Returns the envelope and whether this was the `BuildFinished`
            // that ends an attempt bazel is about to retry.
            let mut process =
                |frame: Frame,
                 raw_out: &mut MultiWriter<BufWriter<File>>|
                 -> Result<(Arc<BuildEventEnvelope>, bool), BuildEventStreamError> {
                    let retry = frame.is_retry_marker();
                    let payload = frame.payload_bytes();
                    let mut event = frame.event;
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
                    let encoded = if modified {
                        reencode_buf.clear();
                        event.encode_length_delimited(&mut reencode_buf)?;
                        raw_out.write_all(reencode_buf.as_slice())?;
                        Bytes::from(event.encode_to_vec())
                    } else {
                        raw_out.write_all(&frame.raw)?;
                        payload
                    };
                    Ok((Arc::new(BuildEventEnvelope { event, encoded }), retry))
                };

            let mut reader = Reader::new(pipe);

            for frame in &mut reader {
                let processed = frame
                    .map_err(BuildEventStreamError::IO)
                    .and_then(|frame| process(frame, &mut raw_out));
                let (envelope, retry) = match processed {
                    Ok(processed) => processed,
                    Err(err) => {
                        signal_all(Err(format!("BES read error: {err}")));
                        broadcaster.close();
                        return Err(err);
                    }
                };

                if retry {
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

                // Fan-out to all subscribers (non-blocking)
                broadcaster.send(envelope);
            }

            // The iterator stops only with an end; recorded before the close
            // below so whatever the close wakes can read it.
            let end = reader
                .into_end()
                .unwrap_or_else(|| End::Corrupt("the stream stopped without a reason".into()));
            let _ = thread_end.set(end);
            finish(&mut raw_out)
        });

        Ok(Self {
            handle: Some(handle),
            broadcaster: Some(main_broadcaster),
            end: end_cell,
        })
    }

    /// How the stream ended, once the reader thread has finished.
    pub fn end(&self) -> Option<&End> {
        self.end.get()
    }

    /// Subscribe to the build event stream without history replay.
    ///
    /// This is for internal use by sinks that subscribe at stream creation time
    /// and don't need history replay. Use `subscribe()` for user-facing APIs
    /// where late subscription support is needed.
    pub fn subscribe(&self) -> Subscriber<Arc<BuildEventEnvelope>> {
        self.subscribe_filtered(None)
    }

    /// Subscribe with an optional send-side filter (see
    /// [`Broadcaster::subscribe_filtered`]). A filtered subscriber's buffer
    /// only holds events the filter accepts — the reader thread skips the rest
    /// before they reach this subscriber, so a `kinds=`-scoped AXL iterator
    /// never pays for the event kinds it doesn't consume.
    pub fn subscribe_filtered(
        &self,
        filter: Option<SubscriberFilter<Arc<BuildEventEnvelope>>>,
    ) -> Subscriber<Arc<BuildEventEnvelope>> {
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
    use crate::stream::pid_pair::PidPair;
    use axl_proto::build_event_stream::BuildEvent;
    use std::fs::OpenOptions;
    use std::path::PathBuf;

    use std::time::Duration;

    /// Encode a `BuildEvent` as a length-delimited protobuf record (LEB128 varint + body),
    /// matching the framing the reader expects.
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

    /// Replay a recorded BEP file through the stream with the production
    /// subscriber shape (gRPC sink + AXL iterator + tracing sink) and report
    /// what the fan-out costs.
    ///
    /// Ignored: it needs a recorded stream, e.g. the `bep.binpb` a lint task
    /// uploads. Measure peak RSS from outside, since the interesting number is
    /// how much of the stream is resident, not how fast it decodes:
    ///
    /// ```text
    /// ASPECT_BES_BENCH_FILE=/path/to/bep.binpb \
    ///   /usr/bin/time -l cargo test -p axl-runtime --release \
    ///   bench_stream_fanout -- --ignored --nocapture
    /// ```
    ///
    /// `ASPECT_BES_BENCH_LAG=request|event|bytes|wire` makes the gRPC-shaped
    /// subscriber hold every event until the producer is done — what a backend
    /// that acks slower than the build produces events does to this process —
    /// in each of the shapes the sink could retain: a built publish request,
    /// the decoded event, the serialized request, or the envelope's own wire
    /// bytes (what the sink keeps).
    #[test]
    #[ignore = "benchmark; needs ASPECT_BES_BENCH_FILE"]
    fn bench_stream_fanout() {
        let Ok(input) = std::env::var("ASPECT_BES_BENCH_FILE") else {
            eprintln!("skipped: set ASPECT_BES_BENCH_FILE to a recorded bep.binpb");
            return;
        };
        // What a lagging sink retains; see the doc comment. Empty drains
        // promptly.
        let lag = std::env::var("ASPECT_BES_BENCH_LAG").unwrap_or_default();

        let path = temp_fifo_path();
        let pid = std::process::id();
        let mut stream = spawn_at(path.clone(), pid, pid, vec![]).unwrap();
        let grpc_sub = stream.subscribe();
        let axl_sub = stream.subscribe();
        let tracing_sub = stream.subscribe();
        wait_for_fifo(&path);

        // Mirrors the gRPC sink: every event is re-encoded into a publish
        // request, and every request is retained until the server acks it.
        let grpc = std::thread::spawn(move || {
            let mut held_requests = Vec::new();
            let mut held_events: Vec<BuildEvent> = Vec::new();
            let mut held_bytes: Vec<(i64, Bytes)> = Vec::new();
            let mut n = 0u64;
            while let Ok(envelope) = grpc_sub.recv() {
                let req = crate::bes::client::build_tool::bazel_event_encoded(
                    "bench-build".to_string(),
                    "bench-invocation".to_string(),
                    n as i64 + 1,
                    envelope.encoded.clone(),
                    Default::default(),
                );
                n += 1;
                match lag.as_str() {
                    "request" => held_requests.push(req),
                    "event" => held_events.push(envelope.event.clone()),
                    "bytes" => held_bytes.push((n as i64, Bytes::from(req.encode_to_vec()))),
                    "wire" => held_bytes.push((n as i64, envelope.encoded.clone())),
                    _ => {}
                }
            }
            (
                n,
                held_requests.len() + held_events.len() + held_bytes.len(),
            )
        });
        let axl = std::thread::spawn(move || {
            let mut n = 0u64;
            while axl_sub.recv().is_ok() {
                n += 1;
            }
            n
        });
        let tracing = std::thread::spawn(move || {
            let mut n = 0u64;
            while tracing_sub.recv().is_ok() {
                n += 1;
            }
            n
        });

        let start = std::time::Instant::now();
        let writer = std::thread::spawn(move || {
            let mut src = std::io::BufReader::new(std::fs::File::open(&input).unwrap());
            let mut fifo = OpenOptions::new().write(true).open(&path).unwrap();
            std::io::copy(&mut src, &mut fifo).unwrap()
        });

        let bytes = writer.join().unwrap();
        stream.join().unwrap();
        let (grpc_n, held) = grpc.join().unwrap();
        let axl_n = axl.join().unwrap();
        let tracing_n = tracing.join().unwrap();
        let elapsed = start.elapsed();

        assert_eq!(grpc_n, axl_n, "every subscriber sees the whole stream");
        assert_eq!(axl_n, tracing_n, "every subscriber sees the whole stream");
        eprintln!(
            "replayed {grpc_n} events ({bytes} bytes) in {elapsed:?} \
             = {:.0} events/s, {} requests retained",
            grpc_n as f64 / elapsed.as_secs_f64(),
            held,
        );
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

    /// A stream on `channel` whose quiet windows are short enough for a test.
    /// `server_pid` is the process whose open files decide `held`: this
    /// process owns the FIFO's read end, so passing its own pid reads as "held"
    /// for as long as the stream is open.
    fn spawn_on(
        channel: Channel,
        server_pid: u32,
        client_pid: u32,
        file_sinks: Vec<(String, Arc<FileSignal>)>,
    ) -> BuildEventStream {
        let mut gate = PidPair {
            client: client_pid,
            server: Some(server_pid),
        }
        .gate(channel.path());
        gate.quiet = Duration::from_millis(300);
        gate.quiet_empty = Duration::from_millis(100);
        BuildEventStream::spawn(channel, gate, file_sinks).unwrap()
    }

    /// [`spawn_on`] a FIFO created at `path`.
    fn spawn_at(
        path: PathBuf,
        server_pid: u32,
        client_pid: u32,
        file_sinks: Vec<(String, Arc<FileSignal>)>,
    ) -> io::Result<BuildEventStream> {
        let channel = Channel::at(path)?;
        Ok(spawn_on(channel, server_pid, client_pid, file_sinks))
    }

    /// Spawn a long-lived `sleep` subprocess and return its pid.
    ///
    /// We need a live, external pid for the server that does NOT hold the FIFO
    /// open: using our own pid would read as "held" (we own the read end), which
    /// keeps the stream from ending on EOF.
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

    /// Bazel can exit before it ever opens the BEP file — an unrecognized flag,
    /// a bad startup option — leaving a live daemon that still looks like a
    /// writer on its way. The stream has to end, empty, rather than wait on a
    /// writer that cannot arrive; the caller reports bazel's exit code.
    #[test]
    fn a_writer_that_never_opens_ends_the_stream() {
        let outcome = crate::testing::with_timeout(Duration::from_secs(10), || {
            let path = temp_fifo_path();
            // Server alive (daemons outlive the invocation), invocation gone.
            let mut holder = spawn_pid_holder();
            let mut stream = spawn_at(path, holder.id(), dead_pid(), vec![]).unwrap();
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

    /// The invocation that never opens the file is also never reaped while the
    /// task is still iterating the stream: it is a zombie, not a free pid.
    #[test]
    fn a_zombie_writer_that_never_opens_ends_the_stream() {
        let outcome = crate::testing::with_timeout(Duration::from_secs(10), || {
            let path = temp_fifo_path();
            let mut holder = spawn_pid_holder();
            let zombie = std::process::Command::new("true")
                .spawn()
                .expect("spawn true");
            std::thread::sleep(Duration::from_millis(200));
            let mut stream = spawn_at(path, holder.id(), zombie.id(), vec![]).unwrap();
            let sub = stream.subscribe();
            let joined = stream.join().is_ok();
            let events: Vec<_> = std::iter::from_fn(|| sub.recv().ok()).collect();
            let _ = holder.kill();
            drop(zombie);
            (joined, events.len())
        });
        assert_eq!(outcome, Some((true, 0)));
    }

    /// The server can keep the write end open after the client is gone (a BEP
    /// transport stuck closing). No end-of-file ever arrives, so the stream
    /// has to end on the client's death alone.
    #[test]
    fn a_dead_client_ends_the_stream_while_the_server_holds_the_pipe() {
        let outcome = crate::testing::with_timeout(Duration::from_secs(10), || {
            let path = temp_fifo_path();
            let channel = Channel::at(path.clone()).expect("mkfifo");
            let mut server_pid = spawn_pid_holder();
            let mut client = spawn_pid_holder();
            let (release, held) = std::sync::mpsc::channel::<()>();
            let path_w = path.clone();
            let server = std::thread::spawn(move || {
                let mut f = OpenOptions::new().write(true).open(&path_w).unwrap();
                f.write_all(&encode_event(&make_event(false))).unwrap();
                let _ = held.recv();
            });
            let mut stream = spawn_on(channel, server_pid.id(), client.id(), vec![]);
            let sub = stream.subscribe();
            let first = sub.recv().is_ok();
            // The client exits; the server still holds the write end.
            let _ = client.kill();
            let _ = client.wait();
            let joined = stream.join().is_ok();
            let _ = release.send(());
            let _ = server.join();
            let _ = server_pid.kill();
            (first, joined)
        });
        assert_eq!(
            outcome,
            Some((true, true)),
            "the stream must end on the client's death while the server holds the pipe"
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

        let mut stream = spawn_at(path.clone(), pid, pid, vec![]).unwrap();
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
        assert!(events[0].event.last_message);
    }

    // -------------------------------------------------------------------------
    // Happy path: Bazel sends a complete stream ending with last_message = true
    // -------------------------------------------------------------------------

    #[test]
    fn test_complete_stream_delivers_all_events() {
        let path = temp_fifo_path();
        let mut holder = spawn_pid_holder();
        let pid = holder.id();

        let mut stream = spawn_at(path.clone(), pid, pid, vec![]).unwrap();
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
        assert!(!events[0].event.last_message);
        assert!(!events[1].event.last_message);
        assert!(events[2].event.last_message);
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

        let mut stream = spawn_at(path.clone(), pid, pid, vec![]).unwrap();
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
        assert!(!events[0].event.last_message);
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
        let outcome = crate::testing::with_timeout(Duration::from_secs(10), || {
            let path = temp_fifo_path();
            let mut holder = spawn_pid_holder();

            let mut stream =
                spawn_at(path.clone(), std::process::id(), holder.id(), vec![]).unwrap();
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
        let outcome = crate::testing::with_timeout(Duration::from_secs(10), || {
            let path = temp_fifo_path();
            let mut holder = spawn_pid_holder();

            let mut stream =
                spawn_at(path.clone(), std::process::id(), holder.id(), vec![]).unwrap();
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
            (
                joined,
                events.len(),
                events.first().map(|e| e.event.last_message),
            )
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

        let mut stream = spawn_at(
            path.clone(),
            pid,
            pid,
            vec![(
                sink_path.to_str().unwrap().to_string(),
                Arc::new(FileSignal::new()),
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

        let mut stream = spawn_at(path.clone(), pid, pid, vec![]).unwrap();
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

        // Attempt 2 — try to open the write end with O_NONBLOCK. The stream
        // thread exited, closing the read end and removing the FIFO, so there
        // is nothing to connect to (a FIFO that was still there with no reader
        // would be ENXIO). Either way the open fails at once, without leaving a
        // dangling blocked thread.
        use nix::libc;
        use std::os::unix::fs::OpenOptionsExt;
        let result = OpenOptions::new()
            .write(true)
            .custom_flags(libc::O_NONBLOCK)
            .open(&path);
        assert!(result.is_err(), "attempt 2 open should fail (no reader)");
        assert_eq!(
            result.unwrap_err().raw_os_error(),
            Some(libc::ENOENT),
            "expected ENOENT (the ended stream removed its FIFO)"
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

        let mut stream = spawn_at(path.clone(), pid, pid, vec![]).unwrap();
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
            events[0].event.payload,
            Some(axl_proto::build_event_stream::build_event::Payload::Started(_))
        ));
        assert!(matches!(
            events[1].event.payload,
            Some(axl_proto::build_event_stream::build_event::Payload::Finished(_))
        ));
        // attempt 2
        assert!(matches!(
            events[2].event.payload,
            Some(axl_proto::build_event_stream::build_event::Payload::Started(_))
        ));
        assert!(events[3].event.last_message);
    }

    /// Bazel may set last_message=true on the BuildFinished(REMOTE_CACHE_EVICTED)
    /// event.  The stream must not close on that last_message because a retry is
    /// expected; it should stay open and deliver attempt 2's events normally.
    #[test]
    fn test_transient_retry_with_last_message_on_evicted_event() {
        let path = temp_fifo_path();
        let mut holder = spawn_pid_holder();
        let pid = holder.id();

        let mut stream = spawn_at(path.clone(), pid, pid, vec![]).unwrap();
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
        assert!(events[1].event.last_message); // attempt 1's BuildFinished had last_message=true ...
        assert!(events[3].event.last_message); // ... but only attempt 2's actually closed the stream
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

        let mut stream = spawn_at(
            path.clone(),
            pid,
            pid,
            vec![(
                sink_path.to_str().unwrap().to_string(),
                Arc::new(FileSignal::new()),
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
