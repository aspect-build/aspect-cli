//! The workspace event log as a stream of decoded [`WorkspaceEvent`]s.
//!
//! Bazel writes `--experimental_workspace_rules_log_file` as a regular file
//! that this process tails ([`crate::stream::file`]), so the decoder never
//! slows Bazel down. Where it is written is a [`LogFile`].
//!
//! The decoded channel is a blocking broadcast ring
//! ([`crate::subscription`]): every subscriber sees every event, and the
//! producer waits for the slowest, so the caller releases its subscribers
//! before [`join`] or one that stopped reading would hold the reader forever.
//!
//! [`join`]: WorkspaceEventStream::join

use axl_proto::workspace_log::WorkspaceEvent;
use fibre::CloseError;
use fibre::spmc::Receiver;
use prost::Message;
use std::fmt::Debug;
use std::io::ErrorKind;
use std::path::{Path, PathBuf};
use std::thread;
use std::thread::JoinHandle;
use thiserror::Error;

use super::LogFile;
use crate::delimited;
use crate::stream::file::StartedOver;
use crate::stream::{Done, Gate};
use crate::subscription::{Origin, ring};

#[derive(Error, Debug)]
pub enum WorkspaceEventError {
    #[error("io error: {0}")]
    IO(#[from] std::io::Error),
    #[error("prost decode error: {0}")]
    ProstDecode(#[from] prost::DecodeError),
    #[error("close error: {0}")]
    Close(#[from] CloseError),
}

pub struct WorkspaceEventStream {
    handle: JoinHandle<Result<(), WorkspaceEventError>>,
    /// Where the log is read from, for naming it in errors.
    path: PathBuf,
    /// The ring's origin, sealed once the consumers are wired.
    origin: Origin<WorkspaceEvent>,
}

impl Debug for WorkspaceEventStream {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("WorkspaceEventStream")
            .field("stream", &String::from("hidden"))
            .finish()
    }
}

impl WorkspaceEventStream {
    /// Start the reader on `log`. `gate` decides when the stream is over (see
    /// [`crate::stream`]); for a real bazel it comes from `PidPair::gate`. The
    /// workspace log has no end marker, so the stream ends only through it.
    ///
    /// Clone subscribers with [`receiver`](Self::receiver), then
    /// [`seal`](Self::seal). Nothing is lost to binding after the reader
    /// starts: the stream's own subscriber holds the ring from the first event.
    ///
    /// A file Bazel starts over is read on from the new one. A read error, or
    /// an event that does not decode, fails [`join`](Self::join).
    pub fn spawn(log: LogFile, gate: Gate) -> Self {
        let (mut sender, origin) = ring::<WorkspaceEvent>();
        let path = log.path().to_owned();
        let mut file = log.follow(gate);
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
            path,
            origin,
        }
    }

    /// The log's path.
    pub fn path(&self) -> &Path {
        &self.path
    }

    /// A further subscriber, for a consumer that will drain it. See
    /// [`Origin::receiver`].
    pub fn receiver(&self) -> Option<Receiver<WorkspaceEvent>> {
        self.origin.receiver()
    }

    /// Stop handing out subscribers once every consumer has one. See
    /// [`Origin`] for why.
    pub fn seal(&mut self) {
        self.origin.seal();
    }

    /// Release the stream's own subscriber if it still holds one, then wait for
    /// the reader.
    pub fn join(mut self) -> Result<(), WorkspaceEventError> {
        self.origin.seal();
        self.handle.join().expect("join error")
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::stream::pid_pair::PidPair;
    use crate::subscription::CHANNEL_CAPACITY;
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
        let log = LogFile::plan(None);
        let path = log.path().to_owned();
        let mut gate = PidPair {
            client: client.id(),
            server: None,
        }
        .gate();
        gate.quiet = Duration::from_millis(300);
        gate.quiet_empty = Duration::from_millis(100);
        (path, WorkspaceEventStream::spawn(log, gate))
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

    /// A path someone else asked for is read where it is and left behind.
    #[test]
    fn a_requested_log_is_read_in_place_and_kept() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("asked.log");
        let read_path = path.clone();
        let outcome = crate::testing::with_timeout(Duration::from_secs(10), move || {
            let client = spawn_pid_holder();
            let mut gate = PidPair {
                client: client.id(),
                server: None,
            }
            .gate();
            gate.quiet = Duration::from_millis(300);
            let mut stream =
                WorkspaceEventStream::spawn(LogFile::Requested(read_path.clone()), gate);
            let recv = stream.receiver().unwrap();
            stream.seal();
            std::fs::write(&read_path, encoded(1..=2)).unwrap();
            end(client);
            let got = std::iter::from_fn(|| recv.recv().ok()).count();
            (got, stream.join().is_ok())
        });
        assert_eq!(outcome, Some((2, true)));
        assert!(path.exists(), "a requested log belongs to whoever asked");
    }
}
