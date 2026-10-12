use axl_proto::build_event_stream::BuildEvent;
use prost::Message;
use prost::bytes::Bytes;
use std::fs::File;
use std::io::BufWriter;
use std::io::ErrorKind;
use std::io::Write;
use std::sync::Arc;
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

/// One build event as the reader thread hands it to subscribers: decoded,
/// and with it the exact bytes it was decoded from (re-encoded when redaction
/// changed it). A subscriber that only forwards events — the gRPC sink —
/// takes the bytes and never touches the proto, so a backlog behind a slow
/// backend costs what bazel wrote and no more; subscribers that read fields
/// share the one decode behind the `Arc`.
#[derive(Debug)]
pub struct BuildEventEnvelope {
    pub event: BuildEvent,
    /// The serialized `BuildEvent` message, without its length prefix.
    pub encoded: Bytes,
}

impl BuildEventEnvelope {
    /// The decoded event, moved out when this was the last reference and
    /// cloned otherwise.
    pub fn into_event(this: Arc<Self>) -> BuildEvent {
        match Arc::try_unwrap(this) {
            Ok(envelope) => envelope.event,
            Err(shared) => shared.event.clone(),
        }
    }
}

/// Largest length prefix this reader will honour. A `BuildEvent` is orders of
/// magnitude smaller; a prefix beyond this is a desynced stream rather than a
/// record, and the varint reader will return up to a 64-bit value, so without
/// a bound the length goes straight into a `Vec::resize` and aborts the
/// process on the allocation.
///
/// Sized to bite early rather than to be generous. The cap bounds the
/// allocation but not what follows it: a prefix under the cap is still read
/// through to the end, and bytes that were never a record never arrive, so
/// the reader waits out the rest of the invocation. A smaller cap turns more
/// of that into an immediate, legible error.
const MAX_EVENT_BYTES: usize = 64 * 1024 * 1024;

/// How long to wait between polls of a file Bazel has not written to yet.
/// Short enough to be invisible next to a build, long enough not to spin.
const WRITER_POLL_INTERVAL: std::time::Duration = std::time::Duration::from_millis(10);

/// How long the daemon may keep writing after the client has exited.
///
/// `--build_event_binary_file_upload_mode=fully_async` lets the command return
/// before the file transport has drained its queue, so the tail of the stream —
/// `BuildFinished` and `last_message` among it — can still be unwritten when
/// the client dies. This bounds that patience: a daemon that holds the file
/// open and writes nothing more must not hang the build.
const POST_INVOCATION_GRACE: std::time::Duration = std::time::Duration::from_secs(10);

/// Turns "nothing written yet" into a blocking read, below the
/// length-delimited framing.
///
/// [`galvanize::StreamingFile`] returns `Ok(0)` for "at the current end of the
/// file, try again later", which the framing above cannot tell from a short
/// read.
///
/// Two facts end the stream, and one is not enough. The client exiting says no
/// *new* events will be produced; the daemon closing the file says none are
/// still on their way to disk. Taking the client's death alone truncates the
/// stream silently, because `Build::wait` reaps the client before it joins this
/// reader and the daemon is still flushing. Taking the daemon's close alone
/// hangs on a stale `server_pid`. So the stream ends when the invocation is
/// over *and* the daemon is off the file — or when
/// [`POST_INVOCATION_GRACE`] runs out, whichever comes first.
struct PendingWriterReader {
    inner: galvanize::StreamingFile,
    writer_pid: u32,
    /// When the invocation was first seen to be over; the grace runs from here.
    invocation_ended_at: Option<std::time::Instant>,
}

impl PendingWriterReader {
    /// Wait for Bazel to reopen the file for its next attempt and resume at the
    /// start of it. `Ok(false)` when the invocation ended instead.
    fn await_next_attempt(&mut self) -> io::Result<bool> {
        let writer_pid = self.writer_pid;
        self.inner
            .await_restart(move || !galvanize::is_pid_alive(writer_pid))
    }

    /// Whether the last error came from the file being replaced rather than
    /// from the stream ending. A replacement is an attempt boundary whatever
    /// the events said: another attempt is demonstrably already writing.
    fn was_replaced(&mut self) -> bool {
        self.inner.take_truncated()
    }

    /// Called once the invocation's pid is gone and the file has no more bytes
    /// in it. True when that really is the end rather than the daemon still
    /// flushing — see [`PendingWriterReader`] for why both answers are needed.
    ///
    /// A stale `server_pid` reports no file open and so settles immediately,
    /// which is the behaviour to want: the alternative is waiting out the grace
    /// on every build whose daemon has since been replaced.
    fn invocation_is_settled(&mut self) -> io::Result<bool> {
        if !self.inner.holder_has_file()? {
            return Ok(true);
        }
        let since = *self
            .invocation_ended_at
            .get_or_insert_with(std::time::Instant::now);
        Ok(since.elapsed() >= POST_INVOCATION_GRACE)
    }
}

impl Read for PendingWriterReader {
    /// Ends the stream only once the invocation is over, not merely because
    /// the holder has no the file open at this instant.
    ///
    /// Those are different questions for an event file, and the gap between
    /// them is where every attempt boundary lives: bazel closes the file
    /// between attempts, and before its first event it has not opened the file
    /// at all. The FIFO this replaced could not tell them apart either — its
    /// `open` blocked for a writer, which is the same patience by another
    /// mechanism. Treating "closed right now" as the end loses the retry, and
    /// on a daemon that opens the file a moment after the path is minted it
    /// loses the whole build.
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        // `Read` requires an empty buffer to return immediately.
        if buf.is_empty() {
            return Ok(0);
        }
        loop {
            // Sampled before the read, not after: a byte written between the
            // read returning nothing and the question being asked would
            // otherwise be declared never to have existed. `StreamingFile::open`
            // orders its own liveness check this way for the same reason.
            let invocation_over = !galvanize::is_pid_alive(self.writer_pid);
            let read = match self.inner.read(buf) {
                // The holder has closed the file with nothing in it. Before
                // this attempt's first byte that is "not opened yet" — bazel
                // mints the path long before its daemon writes to it — so wait
                // rather than call an unstarted stream a finished one.
                //
                // After a byte has been read it is the end of an attempt, and
                // the loop above has to see it: that is where a retry is told
                // from the end of the build.
                // Swallowed, so the replacement it may have latched is
                // consumed here too — left set, it would be read as the answer
                // to some later, unrelated end of stream.
                Err(e) if e.kind() == ErrorKind::BrokenPipe && self.inner.position() == 0 => {
                    self.inner.take_truncated();
                    0
                }
                other => other?,
            };
            if read > 0 {
                return Ok(read);
            }
            if invocation_over && self.invocation_is_settled()? {
                return Err(io::Error::new(ErrorKind::BrokenPipe, "end of stream"));
            }
            thread::sleep(WRITER_POLL_INTERVAL);
        }
    }
}

#[derive(Debug)]
pub struct BuildEventStream {
    /// Thread handle, stored in Option so we can take() it to join without consuming self.
    handle: Option<JoinHandle<Result<(), BuildEventStreamError>>>,
    broadcaster: Option<Broadcaster<Arc<BuildEventEnvelope>>>,
}

/// Unlinks the event file when the reader thread leaves, however it leaves.
///
/// Bazel's event file holds the command line as bazel received it, before the
/// reader redacts anything — remote headers, env passthrough, URL credentials.
/// Leaving it behind would leave those at rest in a shared temp directory for
/// the life of the machine, so the unlink has to survive a panic in the read
/// loop and not just its `return` paths.
///
/// It covers the reader's lifetime and no longer. A bazel attempt that opens
/// the path after the reader has finished recreates the file, and nothing here
/// removes that one; closing it properly means unlinking again once the client
/// has been reaped.
struct EventFileGuard(PathBuf);

impl Drop for EventFileGuard {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.0);
    }
}

impl BuildEventStream {
    /// Mint the path bazel will write `--build_event_binary_file` to, for the
    /// caller to pass on the command line before spawning. Nothing is created
    /// on disk — bazel writes a regular file, so there is no inode to reserve.
    /// Pair with `spawn` once the caller has the bazel child pid in hand.
    pub fn reserve_path() -> io::Result<PathBuf> {
        Ok(env::temp_dir().join(format!("build-event-out-{}.bin", uuid::Uuid::new_v4())))
    }

    /// Spawn the BES reader thread.
    ///
    /// `server_pid` is the process that holds the event file open while it
    /// writes — for real bazel the server/daemon — so it answers "are more
    /// bytes coming?" and "has the next attempt opened the file?".
    ///
    /// `writer_pid` is the per-invocation pid whose death means no more bytes
    /// are coming — for real bazel the spawned client process
    /// (`Command::spawn().id()`). It answers both "will a writer ever arrive?"
    /// (the file never appearing) and "is another attempt coming?" (the
    /// `BrokenPipe` branch in the read loop). Neither question can be put to
    /// `server_pid`, since the daemon outlives the invocation and so always
    /// looks like a writer still on its way.
    ///
    /// Each `(path, signal)` pair gets the raw event bytes mirrored to `path`;
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
            let _event_file = EventFileGuard(path.clone());
            let outcome = (move || {
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

                let open_file_sinks =
                    |paths: &[String]| -> io::Result<MultiWriter<BufWriter<File>>> {
                        let writers = paths
                            .iter()
                            .map(|p| Ok(BufWriter::new(File::create(p)?)))
                            .collect::<io::Result<Vec<_>>>()?;
                        Ok(MultiWriter { writers })
                    };

                // Point the file sinks at fresh truncated files, so a retry's
                // stream starts clean rather than appended to the attempt it
                // replaced. Both ways a retry is detected need this.
                //
                // Order matters: flush first to empty the BufWriter, then open
                // the new sinks (`File::create` truncates). The assignment
                // drops the old MultiWriter, whose BufWriters are empty by
                // then, so its drop-flush writes nothing after the truncation.
                let reset_file_sinks =
                    |raw_out: &mut MultiWriter<BufWriter<File>>| -> io::Result<()> {
                        let _ = raw_out.flush();
                        *raw_out = open_file_sinks(&raw_file_sink_paths)?;
                        Ok(())
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
                // A regular file bazel writes and this thread tails. `open` polls
                // for the file to appear and gives up once neither pid can still
                // create it; a bazel that rejected its command line never writes
                // one, which is an empty stream rather than a failure.
                let inner = match galvanize::StreamingFile::open_restartable(
                    path, server_pid, writer_pid,
                ) {
                    Ok(f) => f,
                    Err(err) if err.kind() == ErrorKind::BrokenPipe => {
                        return finish(&mut raw_out);
                    }
                    Err(err) => {
                        signal_all(Err(format!("failed to open the build event file: {err}")));
                        broadcaster.close();
                        return Err(BuildEventStreamError::IO(err));
                    }
                };
                let mut reader = PendingWriterReader {
                    inner,
                    writer_pid,
                    invocation_ended_at: None,
                };

                let mut buf: Vec<u8> = Vec::with_capacity(1024 * 5);
                // Initial size for reading a varint
                buf.resize(10, 0);

                // Scratch buffer for re-encoding redacted events. Reused across
                // iterations to avoid per-event allocation. Only populated on the
                // rare modified-event path (the common case writes the original
                // bytes straight through).
                let mut reencode_buf: Vec<u8> = Vec::with_capacity(1024);

                let read_event =
                    |buf: &mut Vec<u8>,
                     reencode_buf: &mut Vec<u8>,
                     raw_out: &mut MultiWriter<BufWriter<File>>,
                     reader: &mut PendingWriterReader|
                     -> Result<Arc<BuildEventEnvelope>, BuildEventStreamError> {
                        let (size, vbuf) = read_varint(reader)?;
                        if size > MAX_EVENT_BYTES {
                            return Err(BuildEventStreamError::IO(io::Error::new(
                                ErrorKind::InvalidData,
                                format!(
                                    "build event length prefix of {size} bytes exceeds the \
                                 {MAX_EVENT_BYTES} byte maximum; the event stream is \
                                 not framed where this reader is reading it"
                                ),
                            )));
                        }
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
                        let encoded = if modified {
                            reencode_buf.clear();
                            event.encode_length_delimited(reencode_buf)?;
                            raw_out.write_all(reencode_buf.as_slice())?;
                            Bytes::from(event.encode_to_vec())
                        } else {
                            raw_out.write(vbuf.as_slice())?;
                            raw_out.write_all(&buf[0..size])?;
                            Bytes::copy_from_slice(&buf[0..size])
                        };
                        Ok(Arc::new(BuildEventEnvelope { event, encoded }))
                    };

                // Set by either of two things: a BuildFinished carrying
                // REMOTE_CACHE_EVICTED (code 39), or the event file being replaced
                // under the reader, which is proof of a next attempt whatever the
                // events said.
                // While true, a BrokenPipe (attempt N's writer closing) is swallowed
                // so the reader waits for the retry's stream instead of finishing.
                // Cleared when the next BuildStarted arrives, meaning the retry has
                // connected and normal EOF handling resumes.
                let mut expecting_retry = false;

                loop {
                    match read_event(&mut buf, &mut reencode_buf, &mut raw_out, &mut reader) {
                        Ok(envelope) => {
                            let last_message = envelope.event.last_message;

                            use axl_proto::build_event_stream::build_event::Payload;
                            match &envelope.event.payload {
                                Some(Payload::Finished(finished)) => {
                                    if finished
                                        .exit_code
                                        .as_ref()
                                        .map(|c| c.code == 39)
                                        .unwrap_or(false)
                                    {
                                        expecting_retry = true;
                                        if let Err(e) = reset_file_sinks(&mut raw_out) {
                                            signal_all(Err(format!(
                                                "failed to reopen file sink(s) for retry: {e}"
                                            )));
                                            broadcaster.close();
                                            return Err(BuildEventStreamError::IO(e));
                                        }
                                    }
                                }
                                Some(Payload::Started(_)) => {
                                    expecting_retry = false;
                                }
                                _ => {}
                            }

                            // Fan-out to all subscribers (non-blocking)
                            broadcaster.send(envelope);

                            if last_message && !expecting_retry {
                                return finish(&mut raw_out);
                            }
                        }
                        Err(BuildEventStreamError::IO(err))
                            if err.kind() == ErrorKind::BrokenPipe =>
                        {
                            // A replacement is proof the next attempt is already
                            // writing, whether or not this one announced a retry —
                            // a reader lagging far enough behind can be overtaken
                            // before it ever reads the verdict that would have said
                            // so. Resume at the start of what is there now rather
                            // than splice the two streams together.
                            if reader.was_replaced() {
                                expecting_retry = true;
                                // Attempt N+1 is already at offset zero, so
                                // whatever of attempt N reached the sinks is a
                                // prefix of a stream nobody will finish.
                                if let Err(e) = reset_file_sinks(&mut raw_out) {
                                    signal_all(Err(format!(
                                        "failed to reopen file sink(s) for retry: {e}"
                                    )));
                                    broadcaster.close();
                                    return Err(BuildEventStreamError::IO(e));
                                }
                            }
                            if expecting_retry {
                                // aspect-build/aspect-cli#1060: a REMOTE_CACHE_EVICTED
                                // BuildFinished is not always followed by a retry.
                                // Bazel may emit it as the last message and then
                                // the invocation ends. If the writer process is
                                // gone, no retry is coming — close gracefully
                                // instead of spinning.
                                //
                                // Bazel closes the event file at the end of every
                                // attempt and reopens it for the next with
                                // O_TRUNC (FileTransport.SequentialWriter.close()),
                                // so the next attempt's bytes land at offset zero
                                // of the same path. Resume there rather than at the
                                // offset this attempt ended on, which would read
                                // from the middle of attempt N+1's first message.
                                //
                                // The wait ends on its own if the invocation dies
                                // without retrying — #1060, where a
                                // REMOTE_CACHE_EVICTED BuildFinished is the last
                                // message rather than the prelude to another go.
                                match reader.await_next_attempt() {
                                    Ok(true) => continue,
                                    Ok(false) => return finish(&mut raw_out),
                                    Err(e) => {
                                        signal_all(Err(format!("BES restart failed: {e}")));
                                        broadcaster.close();
                                        return Err(BuildEventStreamError::IO(e));
                                    }
                                }
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
            })();
            outcome
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

        let path = temp_bep_path();
        let pid = std::process::id();
        let mut stream = BuildEventStream::spawn(path.clone(), pid, pid, vec![]).unwrap();
        let grpc_sub = stream.subscribe();
        let axl_sub = stream.subscribe();
        let tracing_sub = stream.subscribe();

        // Mirrors the gRPC sink: every event is re-encoded into a publish
        // request, and every request is retained until the server acks it.
        let grpc = std::thread::spawn(move || {
            let mut held_requests = Vec::new();
            let mut held_events: Vec<BuildEvent> = Vec::new();
            let mut held_bytes: Vec<(i64, Bytes)> = Vec::new();
            let mut n = 0u64;
            while let Ok(envelope) = grpc_sub.recv() {
                let req = build_event_stream::build_tool::bazel_event_encoded(
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
            let mut sink = File::create(&path).unwrap();
            std::io::copy(&mut src, &mut sink).unwrap()
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

    fn temp_bep_path() -> PathBuf {
        std::env::temp_dir().join(format!("test-bes-{}.bin", uuid::Uuid::new_v4()))
    }

    /// A child process that writes one attempt per `segment`, holding the file
    /// open for the length of each and closing it in between — bazel's
    /// `FileTransport`, in a shell.
    ///
    /// It has to be a separate process. `is_path_open_for_pid` is how the
    /// reader decides whether more bytes are coming, and a reader asking that
    /// question about its own process always sees its own descriptor and
    /// answers yes. The returned child's pid is the holder.
    fn spawn_attempt_writer(path: &PathBuf, segments: &[Vec<u8>]) -> std::process::Child {
        let mut script = String::new();
        for (i, seg) in segments.iter().enumerate() {
            let part = std::env::temp_dir().join(format!(
                "test-bes-seg-{}-{}.bin",
                uuid::Uuid::new_v4(),
                i
            ));
            std::fs::write(&part, seg).expect("stage segment");
            // `exec 3> path` truncates and is held by the shell itself, so the
            // shell is the holder; `exec 3>&-` ends the attempt.
            script.push_str(&format!(
                "sleep 0.2; exec 3> {p}; cat {s} >&3; sleep 0.4; exec 3>&-; ",
                p = path.display(),
                s = part.display()
            ));
        }
        // Outlive the last attempt so the invocation still looks alive while the
        // reader drains it.
        script.push_str("sleep 30");
        std::process::Command::new("sh")
            .arg("-c")
            .arg(&script)
            .spawn()
            .expect("spawn attempt writer")
    }

    /// A child process that writes `pieces` into one attempt, holding the file
    /// open across a pause between each, then closing and lingering.
    ///
    /// The pause is the point: it puts the reader at the end of the file with
    /// the holder still on it, which is the state a real build spends most of
    /// its time in. It has to be a separate process for the same reason
    /// [`spawn_attempt_writer`] does — a reader asking
    /// `is_path_open_for_pid` about its own process always sees its own read
    /// descriptor and answers yes, so a holder of `std::process::id()` makes
    /// the question unaskable and the test vacuous.
    ///
    /// The returned child is both the holder and a live invocation pid.
    fn spawn_gapped_writer(path: &PathBuf, pieces: &[Vec<u8>]) -> std::process::Child {
        let mut script = format!("exec 3> {}; ", path.display());
        for (i, piece) in pieces.iter().enumerate() {
            let part = std::env::temp_dir().join(format!(
                "test-bes-piece-{}-{}.bin",
                uuid::Uuid::new_v4(),
                i
            ));
            std::fs::write(&part, piece).expect("stage piece");
            if i > 0 {
                script.push_str("sleep 0.2; ");
            }
            script.push_str(&format!("cat {} >&3; ", part.display()));
        }
        // Close the file, then outlive the reader draining it.
        script.push_str("exec 3>&-; sleep 30");
        std::process::Command::new("sh")
            .arg("-c")
            .arg(&script)
            .spawn()
            .expect("spawn gapped writer")
    }

    /// Spawn a long-lived `sleep` subprocess and return its pid.
    ///
    /// Stands in for the bazel client: a live invocation pid, so the reader
    /// keeps waiting for more attempts. The *holder* pid is this test process,
    /// which is what actually has the event file open.
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
        let outcome = crate::test::with_timeout(Duration::from_secs(10), || {
            let path = temp_bep_path();
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

    /// A dead *holder* is not the end of the stream. The holder pid is read
    /// from `bazel info server_pid` before the build; the daemon it names can
    /// be gone by the time the reader looks — replaced because the next
    /// invocation used different startup options, shut down by a concurrent
    /// `bazel clean`, or simply a separate short-lived process in the fake.
    /// The client starts another daemon and the build proceeds, so waiting on
    /// the old one would report a clean, empty, successful event stream for a
    /// build that produced every event it should have.
    #[test]
    fn a_dead_holder_does_not_empty_the_stream() {
        let path = temp_bep_path();
        let mut invocation = spawn_pid_holder();
        let invocation_pid = invocation.id();

        // Holder already gone before the reader starts; invocation still alive.
        let mut stream =
            BuildEventStream::spawn(path.clone(), dead_pid(), invocation_pid, vec![]).unwrap();
        let sub = stream.subscribe();

        // A gap between the events, which every real build has — bazel writes
        // them over the length of the build. The reader reaches the end of the
        // file partway through, and with a dead holder pid it has no honest
        // way to tell "this attempt is over" from "nothing yet". It must wait.
        //
        // Without this gap the test passes whatever the reader does, because
        // the only end-of-file it ever sees is before the first byte.
        let path_w = path.clone();
        let writer = std::thread::spawn(move || {
            let mut f = File::create(&path_w).unwrap();
            f.write_all(&encode_event(&make_event(false))).unwrap();
            f.flush().unwrap();
            std::thread::sleep(Duration::from_millis(300));
            f.write_all(&encode_event(&make_event(true))).unwrap();
            f.flush().unwrap();
        });

        writer.join().unwrap();
        stream.join().unwrap();
        let _ = invocation.kill();

        let events: Vec<_> = std::iter::from_fn(|| sub.recv().ok()).collect();
        assert_eq!(
            events.len(),
            2,
            "a dead daemon pid must not cut the stream short at a gap between events"
        );
        assert!(events[1].event.last_message);
    }

    /// A length prefix beyond the cap is reported rather than allocated. Only
    /// reachable from a desynced stream, which is exactly when the number is
    /// not a length — so handing it to `Vec::resize` aborts the process on the
    /// allocation instead of failing the build with something readable.
    #[test]
    fn an_absurd_length_prefix_is_a_stream_error_not_an_allocation() {
        let path = temp_bep_path();
        let mut invocation = spawn_pid_holder();
        let pid = invocation.id();

        let mut stream = BuildEventStream::spawn(path.clone(), pid, pid, vec![]).unwrap();
        let _sub = stream.subscribe();

        {
            // A varint well past MAX_EVENT_BYTES: 0xFF×4 then 0x7F is ~2^35.
            let mut f = File::create(&path).unwrap();
            f.write_all(&[0xFF, 0xFF, 0xFF, 0xFF, 0x7F]).unwrap();
            f.flush().unwrap();
        }

        let err = stream
            .join()
            .expect_err("an absurd prefix must fail the stream");
        let msg = err.to_string();
        assert!(
            msg.contains("exceeds") && msg.contains("maximum"),
            "the error should name the cap, got: {msg}"
        );
        let _ = invocation.kill();
        let _ = std::fs::remove_file(&path);
    }

    /// The event file holds bazel's command line as bazel received it, before
    /// this reader redacts anything, so it must not outlive the build in a
    /// shared temp directory.
    #[test]
    fn the_event_file_is_removed_when_the_stream_ends() {
        let path = temp_bep_path();
        let mut invocation = spawn_pid_holder();
        let pid = invocation.id();

        let mut stream = BuildEventStream::spawn(path.clone(), pid, pid, vec![]).unwrap();
        let _sub = stream.subscribe();

        {
            let mut f = File::create(&path).unwrap();
            f.write_all(&encode_event(&make_event(true))).unwrap();
            f.flush().unwrap();
        }

        stream.join().unwrap();
        let _ = invocation.kill();

        assert!(
            !path.exists(),
            "the raw event file must not be left behind: {}",
            path.display()
        );
    }

    /// The bazel client can return before the daemon has written the tail of the
    /// event file — that is exactly what
    /// `--build_event_binary_file_upload_mode=fully_async` asks for, and
    /// `Build::wait` reaps the client before it joins this reader. The client's
    /// death is not the end of the stream while the daemon still holds the file.
    #[test]
    fn the_client_exiting_does_not_cut_off_a_daemon_still_writing() {
        let path = temp_bep_path();
        let seg1 = std::env::temp_dir().join(format!("seg1-{}.bin", uuid::Uuid::new_v4()));
        let seg2 = std::env::temp_dir().join(format!("seg2-{}.bin", uuid::Uuid::new_v4()));
        std::fs::write(&seg1, encode_event(&make_event(false))).unwrap();
        std::fs::write(&seg2, encode_event(&make_event(true))).unwrap();

        // The daemon: writes event 1, holds the file open across the client's
        // exit, then writes the last_message and closes.
        let mut holder = std::process::Command::new("sh")
            .arg("-c")
            .arg(format!(
                "exec 3> {p}; cat {a} >&3; sleep 1; cat {b} >&3; exec 3>&-; sleep 30",
                p = path.display(),
                a = seg1.display(),
                b = seg2.display()
            ))
            .spawn()
            .unwrap();

        // The client: gone well before the daemon's tail write.
        let mut client = std::process::Command::new("sleep").arg("0.3").spawn().unwrap();

        let mut stream =
            BuildEventStream::spawn(path, holder.id(), client.id(), vec![]).unwrap();
        let sub = stream.subscribe();
        stream.join().unwrap();
        let _ = client.wait();
        let _ = holder.kill();
        let _ = holder.wait();
        let _ = std::fs::remove_file(&seg1);
        let _ = std::fs::remove_file(&seg2);

        let events: Vec<_> = std::iter::from_fn(|| sub.recv().ok()).collect();
        assert_eq!(
            events.len(),
            2,
            "the client exiting must not end a stream the daemon is still writing"
        );
        assert!(events[1].event.last_message);
    }

    /// The unlink is the only thing standing between a redacted stream and the
    /// unredacted command line sitting in /tmp for the life of the machine, so
    /// it has to hold when the read loop unwinds rather than returns.
    #[test]
    fn the_event_file_is_removed_even_if_the_reader_panics() {
        let path = temp_bep_path();
        File::create(&path).unwrap();
        assert!(path.exists());

        let p = path.clone();
        let panicked = std::panic::catch_unwind(move || {
            let _event_file = EventFileGuard(p);
            panic!("the read loop fell over");
        });

        assert!(panicked.is_err(), "the test needs the panic to happen");
        assert!(
            !path.exists(),
            "a panicking reader must still unlink the event file: {}",
            path.display()
        );
    }

    /// The other edge: the reader starts the moment bazel is spawned, but bazel
    /// does not open the BEP file until its JVM is up seconds later. That gap
    /// must not be taken for an invocation that will never write, or every
    /// event of a real build is lost.
    #[test]
    fn a_writer_that_opens_late_still_delivers_its_events() {
        let path = temp_bep_path();
        let mut holder = spawn_pid_holder();
        let pid = holder.id();

        let mut stream = BuildEventStream::spawn(path.clone(), pid, pid, vec![]).unwrap();
        let sub = stream.subscribe();

        let path_w = path.clone();
        let writer = std::thread::spawn(move || {
            // Many poll intervals' worth of "no writer", so the reader cannot
            // pass this test by racing to its first read.
            std::thread::sleep(Duration::from_millis(300));
            let mut f = File::create(&path_w).unwrap();
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
        let path = temp_bep_path();

        // The holder is the process that writes the file, as bazel's daemon
        // is. A `sleep` that never opens it would answer "not open" at every
        // gap between events, and the reader would end the stream at the first
        // one it happened to reach.
        let mut attempt = encode_event(&make_event(false));
        attempt.extend_from_slice(&encode_event(&make_event(false)));
        attempt.extend_from_slice(&encode_event(&make_event(true)));
        let mut writer = spawn_attempt_writer(&path, &[attempt]);
        let pid = writer.id();

        let mut stream = BuildEventStream::spawn(path.clone(), pid, pid, vec![]).unwrap();
        let sub = stream.subscribe();

        stream.join().unwrap();
        let _ = writer.kill();
        let _ = writer.wait();

        let events: Vec<_> = std::iter::from_fn(|| sub.recv().ok()).collect();
        assert_eq!(events.len(), 3);
        assert!(!events[0].event.last_message);
        assert!(!events[1].event.last_message);
        assert!(events[2].event.last_message);
    }

    // -------------------------------------------------------------------------
    // End of file: Bazel closes the event file before sending last_message
    // (e.g. Bazel process killed, SIGTERM, or a transient-error attempt ending)
    // -------------------------------------------------------------------------

    /// The holder closing the file is the end of the stream, even with the
    /// invocation still alive and no `last_message` to say so.
    ///
    /// The holder is a real child that closes the file and then lingers, so the
    /// close is the only thing that can end this. A nominated holder that never
    /// had the file open would end the stream at the first EOF and assert the
    /// same outcome without exercising anything, which is what this test used
    /// to do; the elapsed-time assertion is what keeps it honest, since
    /// ignoring the close turns this from a failure into a 30-second pass.
    #[test]
    fn the_holder_closing_the_file_ends_the_stream() {
        let outcome = crate::test::with_timeout(Duration::from_secs(10), || {
            let path = temp_bep_path();
            let mut writer = spawn_gapped_writer(&path, &[encode_event(&make_event(false))]);

            let mut stream =
                BuildEventStream::spawn(path.clone(), writer.id(), writer.id(), vec![]).unwrap();
            let sub = stream.subscribe();

            let started = std::time::Instant::now();
            // BrokenPipe is a graceful end, so this must be Ok.
            let joined = stream.join().map_err(|e| e.to_string());
            let elapsed = started.elapsed();
            let events: Vec<_> = std::iter::from_fn(|| sub.recv().ok()).collect();
            let _ = writer.kill();
            let _ = writer.wait();
            (
                joined,
                events.len(),
                events.first().map(|e| e.event.last_message),
                elapsed < Duration::from_secs(5),
            )
        });
        assert_eq!(
            outcome,
            Some((Ok(()), 1, Some(false), true)),
            "the holder closing the file must end the stream, promptly"
        );
    }

    // -------------------------------------------------------------------------
    // Idle gaps within one attempt
    //
    // A pause with the file still open is `Ok(0)` from the `StreamingFile`,
    // which is indistinguishable from the end of the stream unless the reader
    // asks who still holds the file. The holder is therefore a real child
    // process in both of these.
    // -------------------------------------------------------------------------

    /// Keeps the stream open across a pause between two records.
    #[test]
    fn an_idle_gap_between_records_keeps_the_stream_open() {
        let outcome = crate::test::with_timeout(Duration::from_secs(10), || {
            let path = temp_bep_path();
            let mut writer = spawn_gapped_writer(
                &path,
                &[
                    encode_event(&make_event(false)),
                    encode_event(&make_event(true)),
                ],
            );

            let mut stream =
                BuildEventStream::spawn(path.clone(), writer.id(), writer.id(), vec![]).unwrap();
            let sub = stream.subscribe();

            let joined = stream.join().map_err(|e| e.to_string());
            let events: Vec<_> = std::iter::from_fn(|| sub.recv().ok()).collect();
            let _ = writer.kill();
            let _ = writer.wait();
            (joined, events.len())
        });
        assert_eq!(
            outcome,
            Some((Ok(()), 2)),
            "a pause with the file still open must not end the stream"
        );
    }

    /// Preserves framing when bazel flushes part-way through a record.
    #[test]
    fn a_record_written_in_two_parts_is_not_desynced() {
        let outcome = crate::test::with_timeout(Duration::from_secs(10), || {
            let path = temp_bep_path();
            let record = encode_event(&make_event(true));
            // Stop one byte short so the pause lands inside the record.
            let split = record.len() - 1;
            let mut writer = spawn_gapped_writer(
                &path,
                &[record[..split].to_vec(), record[split..].to_vec()],
            );

            let mut stream =
                BuildEventStream::spawn(path.clone(), writer.id(), writer.id(), vec![]).unwrap();
            let sub = stream.subscribe();

            let joined = stream.join().map_err(|e| e.to_string());
            let events: Vec<_> = std::iter::from_fn(|| sub.recv().ok()).collect();
            let _ = writer.kill();
            let _ = writer.wait();
            (
                joined,
                events.len(),
                events.first().map(|e| e.event.last_message),
            )
        });
        assert_eq!(
            outcome,
            Some((Ok(()), 1, Some(true))),
            "a record spanning a flush must be delivered whole"
        );
    }

    // -------------------------------------------------------------------------
    // File sink: raw bytes are written to a file sink path alongside events
    // -------------------------------------------------------------------------

    #[test]
    fn test_file_sink_captures_raw_bytes() {
        let path = temp_bep_path();
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

        let events_to_send = vec![make_event(false), make_event(true)];
        let raw_bytes: Vec<u8> = events_to_send.iter().flat_map(encode_event).collect();

        let path_w = path.clone();
        let raw_clone = raw_bytes.clone();
        let writer = std::thread::spawn(move || {
            let mut f = File::create(&path_w).unwrap();
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
    // path. With the event file the sequence is:
    //
    //   Attempt 1: bazel creates the file → writes → closes
    //              → the reader sees the end of it, is not expecting a retry,
    //                closes the broadcaster and exits
    //   Attempt 2: bazel truncates and writes again — but nothing is reading
    //
    // Net effect: only attempt 1's events are visible to subscribers. Unlike a
    // FIFO, the write itself still succeeds; what guarantees attempt 2 is not
    // delivered is that the reader has already finished, so that is what this
    // asserts.
    // -------------------------------------------------------------------------

    #[test]
    fn a_late_attempt_after_the_stream_closed_delivers_nothing() {
        let path = temp_bep_path();
        let mut holder = spawn_pid_holder();
        let pid = holder.id();

        let mut stream = BuildEventStream::spawn(path.clone(), pid, pid, vec![]).unwrap();
        let sub = stream.subscribe();

        // Attempt 1 — the writer closes without last_message and without the
        // REMOTE_CACHE_EVICTED verdict, so no retry is expected.
        {
            let mut f = File::create(&path).unwrap();
            f.write_all(&encode_event(&make_event(false))).unwrap();
            f.flush().unwrap();
        }

        stream.join().unwrap();
        let _ = holder.kill();

        let events: Vec<_> = std::iter::from_fn(|| sub.recv().ok()).collect();
        assert_eq!(events.len(), 1, "only attempt 1 events should be visible");

        // Attempt 2 lands in the file after the reader has gone. The write
        // succeeds — it is an ordinary file — and reaches nobody.
        {
            let mut f = File::create(&path).unwrap();
            f.write_all(&encode_event(&make_event(true))).unwrap();
            f.flush().unwrap();
        }
        assert!(
            sub.recv().is_err(),
            "a closed stream delivers nothing from a later attempt"
        );

        let _ = std::fs::remove_file(&path);
    }

    // -------------------------------------------------------------------------
    // Desired behavior: transient error retry delivers events from both attempts
    //
    // Bazel REMOTE_CACHE_EVICTED retry sequence (from transient_error.md):
    //   Attempt 1: create file → write BuildStarted + BuildFinished(FAILED) → close
    //   Attempt 2: truncate file → write BuildStarted + BuildFinished(SUCCESS) → close
    //
    // The subscriber should see all 4 events in order across both attempts.
    //
    // NOT YET IMPLEMENTED: the stream thread currently exits on the first
    // BrokenPipe (attempt 1 close), closing the read end.  Attempt 2's
    // open(O_WRONLY) then blocks indefinitely waiting for a reader.
    //
    // To make this pass the stream would need to keep reading the file
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
        let path = temp_bep_path();
        // Attempt 1 ends on REMOTE_CACHE_EVICTED; attempt 2 truncates the file
        // and writes a fresh stream over it, which the reader must resume at
        // offset zero rather than where attempt 1 left off.
        let mut attempt1 = encode_event(&make_build_started());
        attempt1.extend_from_slice(&encode_event(&make_build_finished(39, false)));
        let mut attempt2 = encode_event(&make_build_started());
        attempt2.extend_from_slice(&encode_event(&make_build_finished(0, true)));

        let mut writer = spawn_attempt_writer(&path, &[attempt1, attempt2]);
        let pid = writer.id();
        let mut stream = BuildEventStream::spawn(path.clone(), pid, pid, vec![]).unwrap();
        let sub = stream.subscribe();

        stream.join().unwrap();
        let _ = writer.kill();
        let _ = writer.wait();

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
        let path = temp_bep_path();
        // Attempt 1's BuildFinished carries both REMOTE_CACHE_EVICTED and
        // last_message=true; the stream must not close on that last_message.
        let mut attempt1 = encode_event(&make_build_started());
        attempt1.extend_from_slice(&encode_event(&make_build_finished(39, true)));
        let mut attempt2 = encode_event(&make_build_started());
        attempt2.extend_from_slice(&encode_event(&make_build_finished(0, true)));

        let mut writer = spawn_attempt_writer(&path, &[attempt1, attempt2]);
        let pid = writer.id();
        let mut stream = BuildEventStream::spawn(path.clone(), pid, pid, vec![]).unwrap();
        let sub = stream.subscribe();

        stream.join().unwrap();
        let _ = writer.kill();
        let _ = writer.wait();

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
        let path = temp_bep_path();
        let sink_path =
            std::env::temp_dir().join(format!("test-bes-sink-{}.bin", uuid::Uuid::new_v4()));
        let mut writer = spawn_attempt_writer(
            &path,
            &[attempt1_raw(last_message_on_evicted), attempt2_raw()],
        );
        let pid = writer.id();

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

        stream.join().unwrap();
        let _ = writer.kill();
        let _ = writer.wait();

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
