use axl_proto::build_event_stream::BuildEvent;
use prost::Message;
use std::fs::File;
use std::io::BufWriter;
use std::io::ErrorKind;
use std::io::Write;
use std::{env, io};
use std::{
    io::Read,
    path::PathBuf,
    thread::{self, JoinHandle},
};
use thiserror::Error;

use super::broadcaster::{Broadcaster, Subscriber};
use super::util::{MultiWriter, read_varint};

#[derive(Error, Debug)]
pub enum BuildEventStreamError {
    #[error("io error: {0}")]
    IO(#[from] std::io::Error),
    #[error("prost decode error: {0}")]
    ProstDecode(#[from] prost::DecodeError),
}

#[derive(Debug)]
pub struct BuildEventStream {
    /// Thread handle, stored in Option so we can take() it to join without consuming self.
    handle: Option<JoinHandle<Result<(), BuildEventStreamError>>>,
    broadcaster: Option<Broadcaster<BuildEvent>>,
}

impl BuildEventStream {
    pub fn spawn_with_pipe(
        pid: u32,
        raw_file_sink_paths: Vec<String>,
    ) -> io::Result<(PathBuf, Self)> {
        let out = env::temp_dir().join(format!("build-event-out-{}.bin", uuid::Uuid::new_v4()));
        let stream = Self::spawn(out.clone(), pid, raw_file_sink_paths)?;
        Ok((out, stream))
    }

    pub fn spawn(path: PathBuf, pid: u32, raw_file_sink_paths: Vec<String>) -> io::Result<Self> {
        let main_broadcaster = Broadcaster::new();
        let thread_broadcaster = main_broadcaster.clone();
        let handle = thread::spawn(move || {
            let broadcaster = thread_broadcaster;
            let open_file_sinks = |paths: &[String]| -> io::Result<MultiWriter<BufWriter<File>>> {
                let writers = paths
                    .iter()
                    .map(|p| Ok(BufWriter::new(File::create(p)?)))
                    .collect::<io::Result<Vec<_>>>()?;
                Ok(MultiWriter { writers })
            };

            let mut raw_out = open_file_sinks(&raw_file_sink_paths)?;
            let mut reader = galvanize::Pipe::new(path, galvanize::RetryPolicy::IfOpenForPid(pid))?;

            let mut buf: Vec<u8> = Vec::with_capacity(1024 * 5);
            // Initial size for reading a varint
            buf.resize(10, 0);

            let read_event = |buf: &mut Vec<u8>,
                              raw_out: &mut MultiWriter<BufWriter<File>>,
                              reader: &mut galvanize::Pipe|
             -> Result<BuildEvent, BuildEventStreamError> {
                let (size, vbuf) = read_varint(reader)?;
                if size > buf.len() {
                    buf.resize(size, 0);
                }
                raw_out.write(vbuf.as_slice())?;
                reader.read_exact(&mut buf[0..size])?;
                // These can be extremely slow and expensive calls depending
                // on the destination that we are writing to.
                // TODO: Ensure we have a dedicated thread where the writing
                // happens to avoid stalling.
                raw_out.write_all(&buf[0..size])?;
                let event = BuildEvent::decode(&buf[0..size])?;
                Ok(event)
            };

            // Set when BuildFinished arrives with REMOTE_CACHE_EVICTED (code 39).
            // While true, a BrokenPipe (attempt N's writer closing) is swallowed
            // so the FIFO read end stays open for the retry writer to reconnect.
            // Cleared when the next BuildStarted arrives, meaning the retry has
            // connected and normal EOF handling resumes.
            let mut expecting_retry = false;

            loop {
                match read_event(&mut buf, &mut raw_out, &mut reader) {
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
                            broadcaster.close();
                            raw_out.flush()?;
                            return Ok(());
                        }
                    }
                    Err(BuildEventStreamError::IO(err)) if err.kind() == ErrorKind::BrokenPipe => {
                        if expecting_retry {
                            // Attempt N's writer closed; Bazel is retrying.
                            // With no writer attached, read() on the FIFO returns
                            // 0 immediately, so looping without backoff creates a
                            // hot CPU spin until the next writer opens the pipe.
                            // Sleep briefly to yield the CPU between polls.
                            std::thread::sleep(std::time::Duration::from_millis(10));
                            continue;
                        }
                        broadcaster.close();
                        raw_out.flush()?;
                        return Ok(());
                    }
                    Err(err) => {
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
        match self.broadcaster.as_ref() {
            Some(b) => b.subscribe(),
            None => {
                // Stream has already been joined; return an immediately-disconnected channel.
                let (tx, rx) = std::sync::mpsc::channel();
                drop(tx);
                rx
            }
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

    /// Poll until the FIFO path exists (created by `galvanize::Pipe::new` in the stream thread).
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

    fn temp_fifo_path() -> PathBuf {
        std::env::temp_dir().join(format!("test-bes-{}.fifo", uuid::Uuid::new_v4()))
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

    // -------------------------------------------------------------------------
    // Happy path: Bazel sends a complete stream ending with last_message = true
    // -------------------------------------------------------------------------

    #[test]
    fn test_complete_stream_delivers_all_events() {
        let path = temp_fifo_path();
        let mut holder = spawn_pid_holder();
        let pid = holder.id();

        let mut stream = BuildEventStream::spawn(path.clone(), pid, vec![]).unwrap();
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

        let mut stream = BuildEventStream::spawn(path.clone(), pid, vec![]).unwrap();
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
            vec![sink_path.to_str().unwrap().to_string()],
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

        let mut stream = BuildEventStream::spawn(path.clone(), pid, vec![]).unwrap();
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

        let mut stream = BuildEventStream::spawn(path.clone(), pid, vec![]).unwrap();
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

        let mut stream = BuildEventStream::spawn(path.clone(), pid, vec![]).unwrap();
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
            vec![sink_path.to_str().unwrap().to_string()],
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
