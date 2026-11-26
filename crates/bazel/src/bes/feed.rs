//! The build event stream of one bazel command: a reader thread follows
//! `--build_event_binary_file` ([`super::binary_file`]), redacts each event,
//! caps its size ([`super::cap`]), mirrors the bytes to file sinks (a trimmed
//! one through [`Trim`]) and fans the events out to subscribers, which bind
//! between [`BuildEventStream::new`] and [`BuildEventStream::start`].

use prost::Message;
use prost::bytes::Bytes;
use std::fs::File;
use std::io::BufWriter;
use std::io::Write;
use std::sync::{Arc, Condvar, Mutex, OnceLock};
use std::thread::{self, JoinHandle};
use thiserror::Error;

use super::BuildEventEnvelope;
use super::binary_file::{End, Reader};
use super::cap::cap_event;
use super::redact::redact_event;
use super::sink::downstream::Trim;
use crate::fanout::{Broadcaster, Subscriber, SubscriberFilter};
use crate::stream::Gate;
use crate::stream::file::Reservation;

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

/// A file a stream is mirrored to, and the signal set once it is whole.
pub struct FileSink {
    pub path: String,
    pub signal: Arc<FileSignal>,
    /// Write the stream through a [`Trim`], as it would have been without the
    /// actions the CLI asked Bazel for, instead of every event.
    pub trim: bool,
    /// Flush after every event, for a reader following the file as it is
    /// written; otherwise it is written a buffer at a time.
    pub flush_each: bool,
}

impl FileSink {
    /// A sink of every event.
    pub fn new(path: String, signal: Arc<FileSignal>) -> Self {
        Self {
            path,
            signal,
            trim: false,
            flush_each: false,
        }
    }
}

/// The file sinks a stream mirrors its raw bytes to.
///
/// A sink that fails to open, write or flush is dropped with a warning and the
/// failure signalled to whoever waits on it, and the others go on: one bad path
/// costs that copy, not the stream.
///
/// TODO: write the sinks on a thread of their own, so a slow destination
/// cannot stall the reader.
struct Mirrors {
    sinks: Vec<Mirror>,
}

struct Mirror {
    path: String,
    signal: Arc<FileSignal>,
    /// `None` before the first open, once failed, and once finished.
    out: Option<BufWriter<File>>,
    /// Set for a trimmed sink, which is written the frames it lets through
    /// instead of the raw bytes.
    trim: Option<Trim>,
    /// [`FileSink::flush_each`].
    flush_each: bool,
}

impl Mirror {
    /// Drop this sink: warn, and signal `why` to whoever waits on it.
    fn fail(&mut self, why: String) {
        self.out = None;
        super::sink::warn(&format!("BES file sink {}: {why}", self.path));
        self.signal.complete(Err(why));
    }

    /// Only [`fail`](Self::fail) completes the signal before the stream ends.
    fn failed(&self) -> bool {
        self.signal.is_complete()
    }

    /// Write one event's `bytes`, flushed with it for a sink that is
    /// followed ([`FileSink::flush_each`]), so its reader has each event as
    /// it comes, not a buffer's worth later.
    fn write(&mut self, bytes: &[u8]) {
        let flush = self.flush_each;
        let wrote = self.out.as_mut().map(|out| {
            out.write_all(bytes)
                .and_then(|()| if flush { out.flush() } else { Ok(()) })
        });
        if let Some(Err(e)) = wrote {
            self.fail(format!("write failed: {e}"));
        }
    }

    /// Write what the trim lets through, each event length-delimited.
    fn write_trimmed(
        &mut self,
        push: impl FnOnce(&mut Trim, &mut dyn FnMut(Arc<BuildEventEnvelope>)),
    ) {
        if self.out.is_none() {
            return;
        }
        let Some(mut trim) = self.trim.take() else {
            return;
        };
        let mut frame = Vec::new();
        push(&mut trim, &mut |envelope| {
            prost::encoding::encode_varint(envelope.encoded.len() as u64, &mut frame);
            frame.extend_from_slice(&envelope.encoded);
        });
        self.trim = Some(trim);
        if !frame.is_empty() {
            self.write(&frame);
        }
    }
}

impl Mirrors {
    fn open(sinks: Vec<FileSink>) -> Self {
        let mut mirrors = Self {
            sinks: sinks
                .into_iter()
                .map(|sink| Mirror {
                    path: sink.path,
                    signal: sink.signal,
                    out: None,
                    trim: sink.trim.then(Trim::new),
                    flush_each: sink.flush_each,
                })
                .collect(),
        };
        mirrors.restart();
        mirrors
    }

    /// Start every sink over, empty: at the start, and at each attempt of a
    /// build bazel retries.
    fn restart(&mut self) {
        for m in self.sinks.iter_mut().filter(|m| !m.failed()) {
            // Flushed before the drop, so the drop writes nothing after
            // `File::create` has truncated the file.
            if let Some(Err(e)) = m.out.as_mut().map(|out| out.flush()) {
                m.fail(format!("flush failed: {e}"));
                continue;
            }
            m.out = None;
            if m.trim.is_some() {
                m.trim = Some(Trim::new());
            }
            match File::create(&m.path) {
                Ok(f) => m.out = Some(BufWriter::new(f)),
                Err(e) => m.fail(format!("cannot open: {e}")),
            }
        }
    }

    /// Write one event, `bytes` being its length-delimited frame.
    fn write(&mut self, bytes: &[u8], envelope: &Arc<BuildEventEnvelope>) {
        for m in &mut self.sinks {
            if m.trim.is_some() {
                m.write_trimmed(|trim, out| trim.push(envelope.clone(), &mut |e| out(e)));
            } else {
                m.write(bytes);
            }
        }
    }

    /// Flush every sink and signal `result` to those still whole.
    fn finish(&mut self, result: Result<(), String>) {
        for m in &mut self.sinks {
            m.write_trimmed(|trim, out| trim.flush(&mut |e| out(e)));
            match m.out.take().map(|mut out| out.flush()) {
                Some(Err(e)) => m.fail(format!("flush failed: {e}")),
                _ => m.signal.complete(result.clone()),
            }
        }
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
    /// The reader thread: `None` until [`start`](Self::start), and again once
    /// joined.
    handle: Option<JoinHandle<Result<(), BuildEventStreamError>>>,
    /// `None` once joined; subscribing then yields a disconnected subscriber.
    broadcaster: Option<Broadcaster<Arc<BuildEventEnvelope>>>,
    /// How the stream ended, set by the reader thread on a clean finish before
    /// it closes the broadcaster, so anything woken by the close can read it.
    /// `None` when the thread returned an error instead.
    end: Arc<OnceLock<End>>,
    /// The size [`cap_event`] holds each event to.
    event_limit: usize,
}

impl BuildEventStream {
    /// Reserve the path the BEP is written to, so it can go on the command
    /// line as `--build_event_binary_file` before bazel is spawned. Bazel
    /// creates the file; the reader removes it once the stream is over.
    pub fn reserve() -> Reservation {
        Reservation::temp("build-event-out")
    }

    /// A stream nothing reads yet. Subscribe, then [`start`](Self::start):
    /// the broadcaster does not replay, and the file keeps whatever bazel
    /// writes in the meantime, so subscribing first is what makes every
    /// subscriber see the whole stream.
    pub fn new() -> Self {
        Self {
            handle: None,
            broadcaster: Some(Broadcaster::new()),
            end: Arc::new(OnceLock::new()),
            event_limit: super::cap::MAX_EVENT_BYTES,
        }
    }

    /// Hold events to what fits in a message of `max_message_bytes`, the
    /// largest the stream's BES backends accept (see [`super::cap::event_limit`]),
    /// instead of [`super::cap::DEFAULT_MAX_MESSAGE_BYTES`].
    pub fn max_message_bytes(mut self, max_message_bytes: usize) -> Self {
        self.event_limit = super::cap::event_limit(max_message_bytes);
        self
    }

    /// Start the reader thread on `file`.
    ///
    /// `gate` answers the two questions that decide when the stream is over
    /// (see [`crate::stream`]); for a real bazel it comes from
    /// `PidPair::gate`, which keys "ended" on the client and "held" on the
    /// server.
    ///
    /// Each [`FileSink`] gets the raw event bytes mirrored to its `path`, or,
    /// trimmed, the frames its [`Trim`] lets through; `signal.complete(result)`
    /// fires after flush, unblocking the file sink handle's `wait()`. A sink
    /// starts over with each attempt of a build bazel retries, so it holds
    /// only the last attempt.
    ///
    /// Every event is redacted (see [`super::redact`]) before anything sees
    /// it: subscribers and file sinks read the same post-redact stream, so
    /// secrets never leave this process. Then [`cap_event`] holds each to the
    /// stream's limit (see [`max_message_bytes`](Self::max_message_bytes)),
    /// with a warning for every event over it,
    /// and every consumer gets the capped copy. An event neither changes, the
    /// common case, is mirrored as its original bytes without a re-encode.
    ///
    /// File sinks are written on the reader thread, so a slow sink slows the
    /// reader, though never bazel.
    pub fn start(mut self, file: Reservation, gate: Gate, file_sinks: Vec<FileSink>) -> Self {
        assert!(self.handle.is_none(), "a stream is started once");
        let thread_broadcaster = self
            .broadcaster
            .clone()
            .expect("a stream is started before it is joined");
        let thread_end = self.end.clone();
        let event_limit = self.event_limit;
        let file = file.follow(gate);
        self.handle = Some(thread::spawn(move || {
            let broadcaster = thread_broadcaster;
            let mut mirrors = Mirrors::open(file_sinks);
            // Reused for the rare event redaction or the cap changes.
            let mut reencode_buf: Vec<u8> = Vec::with_capacity(1024);
            let mut reader = Reader::new(file);

            for frame in &mut reader {
                let frame = match frame {
                    Ok(frame) => frame,
                    Err(err) => {
                        mirrors.finish(Err(format!("BES read error: {err}")));
                        broadcaster.close();
                        return Err(BuildEventStreamError::IO(err));
                    }
                };
                if frame.starts_retried_attempt() {
                    mirrors.restart();
                }

                let payload = frame.payload_bytes();
                let mut event = frame.event;
                let mut changed = redact_event(&mut event);
                let len = if changed {
                    event.encoded_len()
                } else {
                    payload.len()
                };
                if let Some(capped) = cap_event(&mut event, len, event_limit) {
                    super::sink::warn(&capped.warning());
                    changed |= capped.changed();
                }
                let (encoded, frame_bytes) = if changed {
                    reencode_buf.clear();
                    if let Err(err) = event.encode_length_delimited(&mut reencode_buf) {
                        mirrors.finish(Err(format!("BES encode error: {err}")));
                        broadcaster.close();
                        return Err(err.into());
                    }
                    (Bytes::from(event.encode_to_vec()), &reencode_buf[..])
                } else {
                    (payload, &frame.raw[..])
                };
                let envelope = Arc::new(BuildEventEnvelope { event, encoded });
                mirrors.write(frame_bytes, &envelope);
                broadcaster.send(envelope);
            }

            // The iterator stops only with an end; recorded before the close
            // below so whatever the close wakes can read it.
            let end = reader
                .into_end()
                .unwrap_or_else(|| End::Corrupt("the stream stopped without a reason".into()));
            let _ = thread_end.set(end);
            // Says nothing about whether the build succeeded: that is the
            // caller's to read off bazel's exit code.
            broadcaster.close();
            mirrors.finish(Ok(()));
            Ok(())
        }));
        self
    }

    /// How the stream ended, once the reader thread has finished.
    pub fn end(&self) -> Option<&End> {
        self.end.get()
    }

    /// A subscriber that sees every event sent from now on. Subscribe before
    /// [`start`](Self::start) to see the whole stream; the broadcaster does
    /// not replay.
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

    /// Disconnect every subscriber and wait for the reader thread, if it was
    /// started.
    pub fn join(&mut self) -> Result<(), BuildEventStreamError> {
        let _ = self.broadcaster.take();
        if let Some(handle) = self.handle.take() {
            handle.join().expect("join error")?;
        }
        Ok(())
    }
}

impl Default for BuildEventStream {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::bes::testing::{
        action, build_finished, encoded, encoded_all, plain, retry_marker, started,
    };
    use crate::fanout::Subscriber;
    use crate::stream::file::moved_name;
    use crate::stream::pid_pair::PidPair;
    use crate::stream::testing::test_windows;
    use axl_proto::build_event_stream::BuildEvent;
    use axl_proto::build_event_stream::build_event::Payload;
    use std::fs::OpenOptions;
    use std::path::{Path, PathBuf};
    use std::process::{Child, Command, Stdio};
    use std::time::Duration;

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
    ///   /usr/bin/time -l cargo test -p bazel --release \
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

        let mut client = sleeper();
        let file = BuildEventStream::reserve();
        let path = file.path().to_path_buf();
        let stream = BuildEventStream::new();
        let grpc_sub = stream.subscribe();
        let axl_sub = stream.subscribe();
        let tracing_sub = stream.subscribe();
        let gate = gate(client.id(), client.id());
        let mut stream = stream.start(file, gate, vec![]);

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
        let count = |sub: Subscriber<Arc<BuildEventEnvelope>>| {
            std::thread::spawn(move || {
                let mut n = 0u64;
                while sub.recv().is_ok() {
                    n += 1;
                }
                n
            })
        };
        let axl = count(axl_sub);
        let tracing = count(tracing_sub);

        let start = std::time::Instant::now();
        let bytes = std::fs::copy(&input, &path).unwrap();
        let _ = client.kill();
        let _ = client.wait();
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

    /// A long-lived process that holds no file open: the client, or a server
    /// whose pid is stale.
    fn sleeper() -> Child {
        Command::new("sleep")
            .arg("60")
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .expect("spawn sleep")
    }

    /// A server that holds `path` open for writing, as bazel's does while it
    /// writes the stream.
    fn holder_of(path: &Path) -> Child {
        let child = Command::new("sh")
            .arg("-c")
            .arg(r#"exec 3>>"$0"; sleep 60"#)
            .arg(path)
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .expect("spawn holder");
        let deadline = std::time::Instant::now() + Duration::from_secs(5);
        while !path.exists() {
            assert!(std::time::Instant::now() < deadline, "holder never opened");
            std::thread::sleep(Duration::from_millis(5));
        }
        child
    }

    /// A pid that is certainly gone: spawn the shortest-lived process there is
    /// and reap it, so `is_pid_alive` sees a free pid rather than a zombie.
    fn dead_pid() -> u32 {
        let mut child = Command::new("true").spawn().expect("spawn true");
        let pid = child.id();
        child.wait().expect("reap true");
        pid
    }

    /// Write to the file at `path` before the stream starts: once it has,
    /// the reader has moved the file away from the path.
    fn append(path: &Path, bytes: &[u8]) {
        OpenOptions::new()
            .append(true)
            .open(path)
            .unwrap()
            .write_all(bytes)
            .unwrap();
    }

    fn stop(mut child: Child) {
        let _ = child.kill();
        let _ = child.wait();
    }

    fn gate(server: u32, client: u32) -> Gate {
        let mut gate = PidPair {
            client,
            server: Some(server),
        }
        .gate();
        test_windows(&mut gate);
        gate
    }

    /// A stream under test, with one subscriber bound before it starts.
    struct Rig {
        stream: BuildEventStream,
        sub: Subscriber<Arc<BuildEventEnvelope>>,
        path: PathBuf,
        /// The current attempt's file, as bazel holds it.
        writer: std::cell::RefCell<Option<File>>,
    }

    impl Rig {
        fn new(server: u32, client: u32) -> Self {
            Self::with_sinks(server, client, vec![])
        }

        fn with_sinks(server: u32, client: u32, sinks: Vec<(String, Arc<FileSignal>)>) -> Self {
            let sinks = sinks
                .into_iter()
                .map(|(path, signal)| FileSink::new(path, signal))
                .collect();
            Self::of(BuildEventStream::new(), server, client, sinks)
        }

        fn of(stream: BuildEventStream, server: u32, client: u32, sinks: Vec<FileSink>) -> Self {
            let file = BuildEventStream::reserve();
            let path = file.path().to_path_buf();
            let sub = stream.subscribe();
            let stream = stream.start(file, gate(server, client), sinks);
            Self {
                stream,
                sub,
                path,
                writer: Default::default(),
            }
        }

        /// A new attempt, as bazel opens one: the file created or truncated.
        fn attempt(&self, events: &[BuildEvent]) {
            let mut f = File::create(&self.path).unwrap();
            for e in events {
                f.write_all(&encoded(e)).unwrap();
            }
            *self.writer.borrow_mut() = Some(f);
        }

        /// More of the current attempt.
        fn append(&self, bytes: &[u8]) {
            let mut writer = self.writer.borrow_mut();
            writer
                .as_mut()
                .expect("an attempt to append to")
                .write_all(bytes)
                .unwrap();
        }

        fn recv(&self, n: usize) -> Vec<BuildEvent> {
            (0..n)
                .map(|_| self.sub.recv().expect("an event").event.clone())
                .collect()
        }

        /// Join, then everything the subscriber has not taken yet.
        fn finish(mut self) -> (Result<(), String>, Vec<BuildEvent>, PathBuf) {
            let joined = self.stream.join().map_err(|e| e.to_string());
            let rest = std::iter::from_fn(|| self.sub.recv().ok())
                .map(|e| e.event.clone())
                .collect();
            (joined, rest, self.path)
        }
    }

    /// What a file sink wrote, removing it.
    fn take_sink(path: &Path) -> Vec<u8> {
        let written = std::fs::read(path).unwrap();
        let _ = std::fs::remove_file(path);
        written
    }

    /// The event file and the `attempts` names the reader moved it to are
    /// all gone.
    fn nothing_left_of(path: &Path, attempts: u32) {
        assert!(!path.exists());
        for n in 1..=attempts {
            let moved = moved_name(path, n);
            assert!(!moved.exists(), "{} is left behind", moved.display());
        }
    }

    /// A file sink's path, and the spec `start` takes for it.
    fn file_sink() -> (PathBuf, (String, Arc<FileSignal>)) {
        let path = std::env::temp_dir().join(format!("test-bes-sink-{}.bin", uuid::Uuid::new_v4()));
        let spec = (
            path.to_str().unwrap().to_string(),
            Arc::new(FileSignal::new()),
        );
        (path, spec)
    }

    // -------------------------------------------------------------------------
    // Bazel exited before it created the event file
    // -------------------------------------------------------------------------

    /// Bazel can exit before it ever creates the event file (an unrecognized
    /// flag, a bad startup option), leaving a live daemon behind. The stream
    /// has to end, empty, rather than wait for a file that cannot come.
    #[test]
    fn a_file_that_never_appears_ends_the_stream_once_the_client_is_gone() {
        let outcome = crate::testing::with_timeout(Duration::from_secs(10), || {
            let server = sleeper();
            let rig = Rig::new(server.id(), dead_pid());
            let (joined, events, _) = rig.finish();
            stop(server);
            (joined, events.len())
        });
        assert_eq!(outcome, Some((Ok(()), 0)));
    }

    /// The client that never creates the file is also not reaped while the
    /// task is still iterating the stream: it is a zombie, not a free pid.
    #[test]
    fn a_zombie_client_ends_the_stream_too() {
        let outcome = crate::testing::with_timeout(Duration::from_secs(10), || {
            let server = sleeper();
            let zombie = Command::new("true").spawn().expect("spawn true");
            std::thread::sleep(Duration::from_millis(200));
            let rig = Rig::new(server.id(), zombie.id());
            let (joined, events, _) = rig.finish();
            stop(server);
            drop(zombie);
            (joined, events.len())
        });
        assert_eq!(outcome, Some((Ok(()), 0)));
    }

    /// The other edge: bazel creates the file only once its JVM is up, seconds
    /// after the reader starts. That gap is not a build that will never write.
    #[test]
    fn a_file_created_late_still_delivers_its_events() {
        let outcome = crate::testing::with_timeout(Duration::from_secs(10), || {
            let client = sleeper();
            let rig = Rig::new(client.id(), client.id());
            std::thread::sleep(Duration::from_millis(300));
            rig.attempt(&[plain(true)]);
            let (joined, events, _) = rig.finish();
            stop(client);
            (joined, events)
        });
        assert_eq!(outcome, Some((Ok(()), vec![plain(true)])));
    }

    // -------------------------------------------------------------------------
    // How a stream ends
    // -------------------------------------------------------------------------

    #[test]
    fn a_complete_stream_delivers_every_event() {
        let client = sleeper();
        let rig = Rig::new(client.id(), client.id());
        rig.attempt(&[plain(false), plain(false), plain(true)]);
        let (joined, events, _) = rig.finish();
        stop(client);
        assert_eq!(joined, Ok(()));
        assert_eq!(events, vec![plain(false), plain(false), plain(true)]);
    }

    /// Bazel killed mid-build: no `last_message`, and the stream ends with the
    /// client.
    #[test]
    fn a_stream_without_its_final_event_ends_with_the_client() {
        let outcome = crate::testing::with_timeout(Duration::from_secs(10), || {
            let client = sleeper();
            let server = sleeper();
            let rig = Rig::new(server.id(), client.id());
            rig.attempt(&[plain(false)]);
            let first = rig.recv(1);
            stop(client);
            let (joined, rest, _) = rig.finish();
            stop(server);
            (joined, first.len() + rest.len())
        });
        assert_eq!(outcome, Some((Ok(()), 1)));
    }

    /// The server can keep the file open after the client is gone (a BEP
    /// transport stuck closing). The stream ends on the quiet window.
    #[test]
    fn a_dead_client_ends_the_stream_while_the_server_holds_the_file() {
        let outcome = crate::testing::with_timeout(Duration::from_secs(10), || {
            let client = sleeper();
            let file = BuildEventStream::reserve();
            let path = file.path().to_path_buf();
            let server = holder_of(&path);
            append(&path, &encoded(&plain(false)));
            let stream = BuildEventStream::new();
            let sub = stream.subscribe();
            let mut stream = stream.start(file, gate(server.id(), client.id()), vec![]);
            let first = sub.recv().is_ok();
            stop(client);
            let joined = stream.join().is_ok();
            stop(server);
            (first, joined)
        });
        assert_eq!(outcome, Some((true, true)));
    }

    /// A server pid gone stale holds nothing, so every pause between events
    /// looks like nobody is writing. That must not end the stream while the
    /// client runs.
    #[test]
    fn a_pause_between_events_does_not_end_the_stream() {
        let outcome = crate::testing::with_timeout(Duration::from_secs(10), || {
            let client = sleeper();
            let stale_server = sleeper();
            let rig = Rig::new(stale_server.id(), client.id());
            rig.attempt(&[plain(false)]);
            let first = rig.recv(1);
            std::thread::sleep(Duration::from_millis(400));
            rig.append(&encoded(&plain(true)));
            let (joined, rest, _) = rig.finish();
            stop(client);
            stop(stale_server);
            (joined, first.len() + rest.len())
        });
        assert_eq!(outcome, Some((Ok(()), 2)));
    }

    /// Bazel flushing part of a record is a short read, not a framing error.
    #[test]
    fn a_record_written_in_two_parts_is_delivered_whole() {
        let outcome = crate::testing::with_timeout(Duration::from_secs(10), || {
            let client = sleeper();
            let rig = Rig::new(client.id(), client.id());
            let record = encoded(&plain(true));
            let split = record.len() - 1;
            rig.attempt(&[]);
            rig.append(&record[..split]);
            std::thread::sleep(Duration::from_millis(200));
            rig.append(&record[split..]);
            let (joined, events, _) = rig.finish();
            stop(client);
            (joined, events)
        });
        assert_eq!(outcome, Some((Ok(()), vec![plain(true)])));
    }

    #[test]
    fn events_written_before_the_stream_starts_reach_every_subscriber() {
        let client = sleeper();
        let file = BuildEventStream::reserve();
        let path = file.path().to_path_buf();
        let mut bazel = File::create(&path).unwrap();
        bazel.write_all(&encoded(&started())).unwrap();
        let stream = BuildEventStream::new();
        let first = stream.subscribe();
        let second = stream.subscribe();
        let mut stream = stream.start(file, gate(client.id(), client.id()), vec![]);
        bazel.write_all(&encoded(&plain(true))).unwrap();
        drop(bazel);
        stream.join().unwrap();
        stop(client);
        for sub in [first, second] {
            let got: Vec<_> = std::iter::from_fn(|| sub.recv().ok())
                .map(|e| e.event.clone())
                .collect();
            assert_eq!(got, vec![started(), plain(true)]);
        }
    }

    #[test]
    fn the_event_file_is_removed_once_the_stream_is_over() {
        let client = sleeper();
        let rig = Rig::new(client.id(), client.id());
        rig.attempt(&[plain(true)]);
        let (joined, _, path) = rig.finish();
        stop(client);
        assert_eq!(joined, Ok(()));
        nothing_left_of(&path, 1);
    }

    #[test]
    fn joining_an_unstarted_stream_disconnects_its_subscribers() {
        let mut stream = BuildEventStream::new();
        let sub = stream.subscribe();
        stream.join().unwrap();
        assert!(sub.recv().is_err());
        assert!(stream.subscribe().recv().is_err(), "and later ones too");
    }

    #[test]
    #[should_panic(expected = "started once")]
    fn a_stream_is_started_once() {
        let client = sleeper();
        let (a, b) = (BuildEventStream::reserve(), BuildEventStream::reserve());
        let (ga, gb) = (
            gate(client.id(), client.id()),
            gate(client.id(), client.id()),
        );
        let _ = BuildEventStream::new()
            .start(a, ga, vec![])
            .start(b, gb, vec![]);
    }

    // -------------------------------------------------------------------------
    // File sinks
    // -------------------------------------------------------------------------

    /// A sink at a path that cannot be opened fails alone: the stream and the
    /// other sinks go on.
    #[test]
    fn a_file_sink_that_cannot_open_costs_only_its_copy() {
        let outcome = crate::testing::with_timeout(Duration::from_secs(10), || {
            let client = sleeper();
            let (good, good_spec) = file_sink();
            let bad_signal = Arc::new(FileSignal::new());
            let bad = (
                "/nonexistent-dir/for/a/sink.bin".to_string(),
                bad_signal.clone(),
            );
            let rig = Rig::with_sinks(client.id(), client.id(), vec![bad, good_spec]);
            let events = [started(), plain(true)];
            rig.attempt(&events);
            let (joined, got, _) = rig.finish();
            stop(client);
            let written = take_sink(&good);
            (
                joined,
                got == events,
                written == encoded_all(&events),
                bad_signal.wait(),
            )
        });
        let (joined, got, written, bad) = outcome.expect("the stream hung");
        assert_eq!(joined, Ok(()));
        assert!(got, "the subscriber saw every event");
        assert!(written, "the good sink holds every event");
        let err = bad.unwrap_err();
        assert!(err.contains("cannot open"), "{err}");
    }

    /// A sink that cannot be opened again for a retried attempt fails alone.
    #[test]
    fn a_file_sink_that_cannot_restart_costs_only_its_copy() {
        let client = sleeper();
        let dir = std::env::temp_dir().join(format!("test-bes-sinkdir-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir(&dir).unwrap();
        let doomed_signal = Arc::new(FileSignal::new());
        let doomed = (
            dir.join("sink.bin").to_str().unwrap().to_string(),
            doomed_signal.clone(),
        );
        let (good, good_spec) = file_sink();
        let rig = Rig::with_sinks(client.id(), client.id(), vec![doomed, good_spec]);
        rig.attempt(&attempt1(false));
        rig.recv(2);
        std::fs::remove_dir_all(&dir).unwrap();
        rig.attempt(&attempt2());
        let (joined, rest, _) = rig.finish();
        stop(client);
        assert_eq!(joined, Ok(()));
        assert_eq!(rest, attempt2());
        assert!(doomed_signal.wait().is_err());
        let written = take_sink(&good);
        assert_eq!(written, encoded_all(&attempt2()));
    }

    /// Retries exhausted: the last attempt ends on a retry marker and none
    /// follows. The sink keeps that attempt, since nothing replaced it, and
    /// the stream is complete.
    #[test]
    fn a_file_sink_keeps_a_last_attempt_that_ended_on_a_retry_marker() {
        let client = sleeper();
        let (sink, spec) = file_sink();
        let mut rig = Rig::with_sinks(client.id(), client.id(), vec![spec]);
        rig.attempt(&attempt1(true));
        rig.recv(2);
        stop(client);
        assert_eq!(rig.stream.join().map_err(|e| e.to_string()), Ok(()));
        assert!(
            matches!(rig.stream.end(), Some(End::Complete)),
            "{:?}",
            rig.stream.end()
        );
        let (joined, _, _) = rig.finish();
        assert_eq!(joined, Ok(()));
        let written = take_sink(&sink);
        assert_eq!(written, encoded_all(&attempt1(true)));
    }

    #[test]
    fn a_file_sink_is_whole_and_empty_when_bazel_never_wrote() {
        let server = sleeper();
        let (sink, (path, signal)) = file_sink();
        let rig = Rig::with_sinks(server.id(), dead_pid(), vec![(path, signal.clone())]);
        let (joined, events, _) = rig.finish();
        stop(server);
        assert_eq!(joined, Ok(()));
        assert!(events.is_empty());
        assert_eq!(signal.wait(), Ok(()));
        assert_eq!(take_sink(&sink), b"");
    }

    #[test]
    fn a_file_sink_captures_the_raw_bytes() {
        let client = sleeper();
        let (sink, spec) = file_sink();
        let rig = Rig::with_sinks(client.id(), client.id(), vec![spec]);
        let events = [plain(false), plain(true)];
        rig.attempt(&events);
        let (joined, _, _) = rig.finish();
        stop(client);
        assert_eq!(joined, Ok(()));
        let written = take_sink(&sink);
        assert_eq!(written, encoded_all(&events));
    }

    fn announcing(labels: &[&str]) -> BuildEvent {
        crate::bes::testing::announcing(plain(false), labels)
    }

    /// A rig of one client mirroring to `sinks`.
    fn sinks_rig(client: u32, sinks: Vec<FileSink>) -> Rig {
        Rig::of(BuildEventStream::new(), client, client, sinks)
    }

    /// A followed sink has each event on disk once the reader has passed
    /// it on; any other is written a buffer at a time.
    #[test]
    fn only_a_followed_file_sink_is_flushed_per_event() {
        let events = [started(), plain(false)];
        let client = sleeper();
        let (followed, (path, signal)) = file_sink();
        let (buffered, (plain_path, plain_signal)) = file_sink();
        let rig = sinks_rig(
            client.id(),
            vec![
                FileSink {
                    flush_each: true,
                    ..FileSink::new(path, signal)
                },
                FileSink::new(plain_path, plain_signal),
            ],
        );
        rig.attempt(&events);
        rig.recv(events.len());
        assert_eq!(std::fs::read(&followed).unwrap(), encoded_all(&events));
        assert!(std::fs::read(&buffered).unwrap().is_empty());
        rig.append(&encoded(&plain(true)));
        let (joined, _, _) = rig.finish();
        stop(client);
        assert_eq!(joined, Ok(()));
        let all = [started(), plain(false), plain(true)];
        assert_eq!(take_sink(&followed), encoded_all(&all));
        assert_eq!(take_sink(&buffered), encoded_all(&all));
    }

    #[test]
    fn a_trimmed_file_sink_loses_only_the_successful_actions() {
        let events = [
            announcing(&["ok", "bad"]),
            action("ok", true),
            action("bad", false),
            plain(true),
        ];
        let client = sleeper();
        let (trimmed, (path, signal)) = file_sink();
        let (whole, (whole_path, whole_signal)) = file_sink();
        let rig = sinks_rig(
            client.id(),
            vec![
                FileSink {
                    path,
                    signal,
                    trim: true,
                    flush_each: false,
                },
                FileSink::new(whole_path, whole_signal),
            ],
        );
        rig.attempt(&events);
        let (joined, seen, _) = rig.finish();
        stop(client);
        assert_eq!(joined, Ok(()));
        assert_eq!(seen.len(), 4, "subscribers see every event");
        assert_eq!(take_sink(&whole), encoded_all(&events));
        assert_eq!(
            take_sink(&trimmed),
            encoded_all(&[announcing(&["bad"]), action("bad", false), plain(true)]),
        );
    }

    #[test]
    fn a_trimmed_file_sink_holds_only_the_retried_attempt() {
        let first = [started(), announcing(&["evicted"]), retry_marker(false)];
        let second = [
            started(),
            announcing(&["ok"]),
            action("ok", true),
            build_finished(0, true),
        ];
        let client = sleeper();
        let (trimmed, (path, signal)) = file_sink();
        let rig = sinks_rig(
            client.id(),
            vec![FileSink {
                path,
                signal,
                trim: true,
                flush_each: false,
            }],
        );
        rig.attempt(&first);
        rig.recv(first.len());
        rig.attempt(&second);
        let (joined, _, _) = rig.finish();
        stop(client);
        assert_eq!(joined, Ok(()));
        assert_eq!(
            take_sink(&trimmed),
            encoded_all(&[started(), announcing(&[]), build_finished(0, true)]),
        );
    }

    // -------------------------------------------------------------------------
    // A build bazel retries after REMOTE_CACHE_EVICTED
    //
    // Each attempt creates the file over with O_TRUNC. Bazel's gap between
    // attempts is a whole loading and analysis phase, which these tests stand
    // in for by letting the reader reach the retry marker before the next
    // attempt starts.
    // -------------------------------------------------------------------------

    fn attempt1(last_message: bool) -> [BuildEvent; 2] {
        [started(), retry_marker(last_message)]
    }

    fn attempt2() -> [BuildEvent; 2] {
        [started(), build_finished(0, true)]
    }

    fn run_retry(
        last_message_on_evicted: bool,
        sinks: Vec<(String, Arc<FileSignal>)>,
    ) -> Vec<BuildEvent> {
        let client = sleeper();
        let rig = Rig::with_sinks(client.id(), client.id(), sinks);
        rig.attempt(&attempt1(last_message_on_evicted));
        let mut events = rig.recv(2);
        rig.attempt(&attempt2());
        let (joined, rest, path) = rig.finish();
        stop(client);
        assert_eq!(joined, Ok(()));
        nothing_left_of(&path, 2);
        events.extend(rest);
        events
    }

    #[test]
    fn a_retry_delivers_both_attempts_in_order() {
        let events = run_retry(false, vec![]);
        assert_eq!(events, [attempt1(false), attempt2()].concat());
    }

    /// Bazel may set `last_message` on the retried attempt's `BuildFinished`.
    /// Only the last attempt's final event ends the stream.
    #[test]
    fn a_retry_with_last_message_on_the_evicted_attempt_delivers_both() {
        let events = run_retry(true, vec![]);
        assert_eq!(events, [attempt1(true), attempt2()].concat());
    }

    fn retry_sink(last_message_on_evicted: bool) -> Vec<u8> {
        let (sink, spec) = file_sink();
        run_retry(last_message_on_evicted, vec![spec]);
        let written = take_sink(&sink);
        written
    }

    /// The file sinks start over with the retry, holding only its bytes.
    #[test]
    fn file_sinks_hold_only_the_retried_stream() {
        let want = encoded_all(&attempt2());
        assert_eq!(retry_sink(false), want);
        assert_eq!(retry_sink(true), want);
    }

    /// A sink that failed to open stays failed across a retry, and the good
    /// sink beside it still starts over for the retried attempt.
    #[test]
    fn a_failed_sink_stays_failed_across_a_retry() {
        let dir = tempfile::tempdir().unwrap();
        let bad_path = dir.path().join("missing-dir").join("bad.bin");
        let bad = Arc::new(FileSignal::new());
        let (good, good_spec) = file_sink();
        run_retry(
            false,
            vec![
                (bad_path.to_str().unwrap().to_string(), bad.clone()),
                good_spec,
            ],
        );
        let written = take_sink(&good);
        let err = bad.wait().unwrap_err();
        assert!(err.contains("cannot open"), "{err}");
        assert!(!bad_path.exists());
        assert_eq!(written, encoded_all(&attempt2()));
    }

    /// An event redaction changes reaches the sink re-encoded, and after a
    /// retry only the retried attempt's copy of it is there.
    #[test]
    fn a_redacted_event_after_a_retry_reaches_the_sink_re_encoded() {
        let with_options = || {
            let mut e = started();
            if let Some(Payload::Started(s)) = e.payload.as_mut() {
                s.options_description = "--repo_env=SECRET=hunter2".into();
            }
            e
        };
        let client = sleeper();
        let (sink, spec) = file_sink();
        let rig = Rig::with_sinks(client.id(), client.id(), vec![spec]);
        rig.attempt(&[with_options(), retry_marker(false)]);
        rig.recv(2);
        rig.attempt(&[with_options(), build_finished(0, true)]);
        let (joined, rest, _) = rig.finish();
        stop(client);
        assert_eq!(joined, Ok(()));
        assert_eq!(rest, vec![started(), build_finished(0, true)], "redacted");
        assert_eq!(
            take_sink(&sink),
            encoded_all(&[started(), build_finished(0, true)])
        );
    }

    /// An action event over the size limit reaches subscribers, their wire
    /// bytes and file sinks with its command line cut to fit; any other event
    /// over it reaches them unchanged.
    #[test]
    fn oversized_events_reach_every_consumer_capped() {
        use axl_proto::build_event_stream::{ActionExecuted, Progress};
        let big_action = BuildEvent {
            payload: Some(Payload::Action(ActionExecuted {
                command_line: vec!["x".repeat(1000); 10_000],
                ..Default::default()
            })),
            ..Default::default()
        };
        let big_progress = BuildEvent {
            payload: Some(Payload::Progress(Progress {
                stderr: "x".repeat(super::super::cap::MAX_EVENT_BYTES),
                ..Default::default()
            })),
            ..Default::default()
        };
        let mut capped = big_action.clone();
        let len = capped.encoded_len();
        assert!(
            cap_event(&mut capped, len, super::super::cap::MAX_EVENT_BYTES)
                .is_some_and(|c| c.changed())
        );
        let client = sleeper();
        let (sink, spec) = file_sink();
        let rig = Rig::with_sinks(client.id(), client.id(), vec![spec]);
        rig.attempt(&[big_action, big_progress.clone(), build_finished(0, true)]);
        let wire: Vec<Bytes> = (0..2)
            .map(|_| rig.sub.recv().expect("an event").encoded.clone())
            .collect();
        let (joined, rest, _) = rig.finish();
        stop(client);
        assert_eq!(joined, Ok(()));
        assert_eq!(
            wire,
            vec![capped.encode_to_vec(), big_progress.encode_to_vec()]
        );
        assert_eq!(rest, vec![build_finished(0, true)]);
        assert_eq!(
            take_sink(&sink),
            encoded_all(&[capped, big_progress, build_finished(0, true)])
        );
    }

    /// A stream whose backends take larger messages passes on an action event
    /// that would be cut under the default.
    #[test]
    fn a_larger_message_size_passes_a_larger_event_on_whole() {
        use axl_proto::build_event_stream::ActionExecuted;
        let big = BuildEvent {
            payload: Some(Payload::Action(ActionExecuted {
                command_line: vec!["x".repeat(1000); 6_000],
                ..Default::default()
            })),
            ..Default::default()
        };
        assert!(big.encoded_len() > super::super::cap::MAX_EVENT_BYTES);
        let client = sleeper();
        let rig = Rig::of(
            BuildEventStream::new().max_message_bytes(16 << 20),
            client.id(),
            client.id(),
            vec![],
        );
        rig.attempt(&[big.clone(), build_finished(0, true)]);
        let (joined, rest, _) = rig.finish();
        stop(client);
        assert_eq!(joined, Ok(()));
        assert_eq!(rest, vec![big, build_finished(0, true)]);
    }

    /// The retried attempt goes on writing after its retry marker. That tail
    /// belongs to the attempt bazel abandoned, not to the sink's last attempt.
    #[test]
    fn file_sinks_drop_the_tail_of_the_retried_attempt() {
        let client = sleeper();
        let (sink, spec) = file_sink();
        let rig = Rig::with_sinks(client.id(), client.id(), vec![spec]);
        // Attempt 1's writer stays open: the reader has moved the file off the
        // path, and the tail is written through the fd bazel already has.
        let mut first = File::create(&rig.path).unwrap();
        for e in attempt1(false) {
            first.write_all(&encoded(&e)).unwrap();
        }
        rig.recv(2);
        first.write_all(&encoded(&plain(true))).unwrap();
        drop(first);
        rig.recv(1);
        rig.attempt(&attempt2());
        let (joined, _, path) = rig.finish();
        stop(client);
        assert_eq!(joined, Ok(()));
        nothing_left_of(&path, 2);
        let written = take_sink(&sink);
        assert_eq!(written, encoded_all(&attempt2()));
    }
}
