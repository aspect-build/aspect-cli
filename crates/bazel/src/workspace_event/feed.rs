//! The workspace event log as a stream of decoded [`WorkspaceEvent`]s.
//!
//! Bazel writes `--experimental_workspace_rules_log_file` as a regular file
//! that this process tails ([`crate::stream::file`]), so the decoder never
//! slows Bazel down. The path is a [`Reservation`]: nothing else writes it, and
//! it is removed once the read is over.
//!
//! The decoded channel is a blocking broadcast ring, as for the execution log:
//! every subscriber sees every event, the producer waits for the slowest, and
//! `build.wait()` releases the AXL-side subscribers before [`join`] so one that
//! stopped reading cannot hold the reader forever.
//!
//! [`join`]: WorkspaceEventStream::join

use axl_proto::workspace_log::WorkspaceEvent;
use fibre::spmc::{Receiver, bounded};
use fibre::{CloseError, SendError};
use prost::Message;
use std::fmt::Debug;
use std::io::ErrorKind;
use std::thread;
use std::thread::JoinHandle;
use thiserror::Error;

use crate::delimited;
use crate::stream::file::{Reservation, StartedOver};
use crate::stream::{Done, Gate};

/// Events the decoded channel buffers per subscriber before the producer waits.
pub const CHANNEL_CAPACITY: usize = 1000;

#[derive(Error, Debug)]
pub enum WorkspaceEventError {
    #[error("io error: {0}")]
    IO(#[from] std::io::Error),
    #[error("prost decode error: {0}")]
    ProstDecode(#[from] prost::DecodeError),
    #[error("send error: {0}")]
    Send(#[from] SendError),
    #[error("close error: {0}")]
    Close(#[from] CloseError),
}

pub struct WorkspaceEventStream {
    handle: JoinHandle<Result<(), WorkspaceEventError>>,
    /// The one subscriber the stream starts with, and the only thing a further
    /// one can be cloned from. Dropped by [`seal`](Self::seal) once consumers
    /// have cloned theirs: left unread beside them it would hold the ring full
    /// at [`CHANNEL_CAPACITY`], and with no consumer at all its absence lets
    /// the reader skip decoding.
    recv: Option<Receiver<WorkspaceEvent>>,
}

impl Debug for WorkspaceEventStream {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("WorkspaceEventStream")
            .field("stream", &String::from("hidden"))
            .finish()
    }
}

impl WorkspaceEventStream {
    /// Reserve the path the workspace log is written to, so it can go on the
    /// command line before bazel is spawned. Pair with [`spawn`](Self::spawn)
    /// once the bazel client exists.
    pub fn reserve() -> Reservation {
        Reservation::temp("workspace-event-out")
    }

    /// Start the reader on `file`. `gate` decides when the stream is over (see
    /// [`crate::stream`]); for a real bazel it comes from `PidPair::gate`. The
    /// workspace log has no end marker, so the stream ends only through it.
    ///
    /// Clone subscribers with [`receiver`](Self::receiver), then
    /// [`seal`](Self::seal). Nothing is lost to binding after the reader
    /// starts: the stream's own subscriber holds the ring from the first event.
    ///
    /// A file Bazel starts over is read on from the new one. A read error, or
    /// an event that does not decode, fails [`join`](Self::join).
    pub fn spawn(file: Reservation, gate: Gate) -> Self {
        let (mut sender, recv) = bounded::<WorkspaceEvent>(CHANNEL_CAPACITY);
        let mut file = file.follow(gate);
        let handle = thread::spawn(move || {
            let mut buf: Vec<u8> = Vec::with_capacity(1024 * 5);
            let mut has_readers = true;
            loop {
                match delimited::next_frame(&mut file, &mut buf) {
                    Ok(Some(header)) => {
                        if has_readers {
                            let event = WorkspaceEvent::decode(&buf[..header.len])?;
                            has_readers = sender.send(event).is_ok();
                        }
                    }
                    // The end of the stream: between two events, or cut off in
                    // the middle of one. Why it ended is the file's to say.
                    Ok(None) => break,
                    Err(err) if StartedOver::is(&err) => continue,
                    Err(err) if err.kind() == ErrorKind::UnexpectedEof => break,
                    Err(err) => return Err(err.into()),
                }
            }
            if let Some(Done::Broken { error, .. }) = file.take_done() {
                return Err(error.into());
            }
            if has_readers {
                sender.close()?;
            }
            Ok(())
        });
        Self {
            handle,
            recv: Some(recv),
        }
    }

    /// A further subscriber, cloned from the stream's own. `None` once that
    /// one has been dropped ([`seal`](Self::seal), `join()`).
    pub fn receiver(&self) -> Option<Receiver<WorkspaceEvent>> {
        self.recv.as_ref().cloned()
    }

    /// Drop the stream's own subscriber once every consumer has cloned one. No
    /// consumer can be added afterwards.
    pub fn seal(&mut self) {
        self.recv.take();
    }

    /// Release the stream's own subscriber if it still holds one, then wait for
    /// the reader.
    pub fn join(mut self) -> Result<(), WorkspaceEventError> {
        self.recv.take();
        self.handle.join().expect("join error")
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::stream::pid_pair::PidPair;
    use std::path::PathBuf;
    use std::process::Child;
    use std::time::Duration;

    fn spawn_pid_holder() -> Child {
        std::process::Command::new("sleep")
            .arg("60")
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .spawn()
            .expect("failed to spawn sleep")
    }

    fn event(n: u32) -> WorkspaceEvent {
        WorkspaceEvent {
            location: n.to_string(),
            ..Default::default()
        }
    }

    fn encoded(events: impl IntoIterator<Item = u32>) -> Vec<u8> {
        events
            .into_iter()
            .flat_map(|n| event(n).encode_length_delimited_to_vec())
            .collect()
    }

    /// A stream on a fresh reservation whose quiet windows suit a test, ended
    /// by `client`'s exit. Returns the path Bazel would write.
    fn spawn_for(client: &Child) -> (PathBuf, WorkspaceEventStream) {
        let file = WorkspaceEventStream::reserve();
        let path = file.path().to_owned();
        let mut gate = PidPair {
            client: client.id(),
            server: None,
        }
        .gate();
        gate.quiet = Duration::from_millis(300);
        gate.quiet_empty = Duration::from_millis(100);
        (path, WorkspaceEventStream::spawn(file, gate))
    }

    fn end(mut client: Child) {
        let _ = client.kill();
        let _ = client.wait();
    }

    /// Writes `bytes` as Bazel would, ends the client, and returns what one
    /// subscriber received and how the stream joined.
    fn read_back(bytes: Vec<u8>) -> (Vec<String>, Result<(), WorkspaceEventError>) {
        crate::testing::with_timeout(Duration::from_secs(10), move || {
            let client = spawn_pid_holder();
            let (path, mut stream) = spawn_for(&client);
            let recv = stream.receiver().unwrap();
            stream.seal();
            std::fs::write(&path, bytes).unwrap();
            end(client);
            let got = std::iter::from_fn(|| recv.recv().ok())
                .map(|e| e.location)
                .collect();
            (got, stream.join())
        })
        .expect("the stream hung")
    }

    #[test]
    fn events_arrive_in_order_and_the_stream_ends_with_the_client() {
        let (got, joined) = read_back(encoded(1..=3));
        assert_eq!(got, ["1", "2", "3"]);
        assert!(joined.is_ok(), "{joined:?}");
    }

    /// Bazel can exit before it writes the workspace log at all.
    #[test]
    fn a_client_that_never_writes_the_log_ends_the_stream() {
        let joined = crate::testing::with_timeout(Duration::from_secs(10), || {
            let client = spawn_pid_holder();
            let (_path, stream) = spawn_for(&client);
            end(client);
            stream.join().is_ok()
        });
        assert_eq!(joined, Some(true));
    }

    /// A log longer than the ring with no subscriber: sealing drops the
    /// stream's own, so the reader stops decoding instead of parking at
    /// [`CHANNEL_CAPACITY`] against a subscriber nobody reads.
    #[test]
    fn a_sealed_stream_with_no_subscriber_reads_past_the_ring() {
        let joined = crate::testing::with_timeout(Duration::from_secs(10), || {
            let client = spawn_pid_holder();
            let (path, mut stream) = spawn_for(&client);
            stream.seal();
            std::fs::write(&path, encoded(0..CHANNEL_CAPACITY as u32 * 3)).unwrap();
            end(client);
            stream.join().is_ok()
        });
        assert_eq!(joined, Some(true));
    }

    #[test]
    fn a_stream_cut_off_inside_an_event_keeps_the_events_before_it() {
        let mut bytes = encoded([1]);
        // A length that promises more than is written.
        bytes.extend([5, 0, 0]);
        let (got, joined) = read_back(bytes);
        assert_eq!(got, ["1"]);
        assert!(joined.is_ok(), "{joined:?}");
    }

    #[test]
    fn an_event_that_does_not_decode_is_an_error() {
        let (_, joined) = read_back(vec![4, 0xff, 0xff, 0xff, 0xff]);
        assert!(
            matches!(joined, Err(WorkspaceEventError::ProstDecode(_))),
            "{joined:?}"
        );
    }

    #[test]
    fn an_absurd_event_length_is_an_error() {
        let (_, joined) = read_back(vec![0x80, 0x80, 0x80, 0x80, 0x80, 0x20]);
        assert!(
            matches!(&joined, Err(WorkspaceEventError::IO(e)) if e.kind() == ErrorKind::InvalidData),
            "{joined:?}"
        );
    }

    /// The reservation owns its path: once the read is over nothing is left
    /// in the temp dir.
    #[test]
    fn the_log_is_removed_once_read() {
        let path = crate::testing::with_timeout(Duration::from_secs(10), || {
            let client = spawn_pid_holder();
            let (path, mut stream) = spawn_for(&client);
            stream.seal();
            std::fs::write(&path, encoded(1..=2)).unwrap();
            end(client);
            stream.join().unwrap();
            path
        })
        .unwrap();
        let name = path.file_name().unwrap().to_string_lossy().to_string();
        assert!(name.starts_with("workspace-event-out-"), "{name}");
        let leftovers: Vec<_> = std::fs::read_dir(path.parent().unwrap())
            .unwrap()
            .filter_map(|e| e.ok())
            .filter(|e| e.file_name().to_string_lossy().starts_with(&name))
            .collect();
        assert!(leftovers.is_empty(), "{leftovers:?}");
    }
}
