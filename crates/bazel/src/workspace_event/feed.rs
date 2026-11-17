//! The workspace event log as a stream of decoded [`WorkspaceEvent`]s.
//!
//! Bazel writes `--experimental_workspace_rules_log_file` as a regular file
//! that this process tails ([`crate::stream::file`]), so the decoder never
//! slows Bazel down. Where it is written is a [`LogFile`], and each path a
//! caller asked for is a copy written as the file is read.
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
use std::io::ErrorKind;
use std::path::{Path, PathBuf};
use std::thread;
use std::thread::JoinHandle;
use thiserror::Error;

use super::LogFile;
use crate::delimited;
use crate::stream::file::StartedOver;
use crate::stream::tee::{MultiTeeReader, Tee, clear_stale};
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
    #[error("{0}")]
    Copy(String),
}

#[derive(Debug)]
pub struct WorkspaceEventStream {
    handle: JoinHandle<Result<(), WorkspaceEventError>>,
    /// Where the log is read from, for naming it in errors.
    path: PathBuf,
    /// The ring's origin, sealed once the consumers are wired.
    origin: Origin<WorkspaceEvent>,
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
    /// A file Bazel starts over is read on from the new one, and the `copies`
    /// start over with it. Once the stream is over they are written again from
    /// Bazel's finished file, so a restart the reader missed cannot leave them
    /// a mix of two attempts. A read error, or an event that does not decode,
    /// fails [`join`](Self::join), as does a copy that could not be written
    /// whole, which costs only itself; an event that does not decode ends the
    /// decoded stream there, and the file is still read to its end. Events are decoded only while a
    /// subscriber is left to receive them, so a log nobody reads is not
    /// validated past the point its readers let go.
    pub fn spawn(log: LogFile, gate: Gate, copies: Vec<String>) -> Self {
        let (mut sender, origin) = ring::<WorkspaceEvent>();
        let path = log.path().to_owned();
        let mut followed = log.follow(gate);
        let handle = thread::spawn(move || {
            // A copy is created once Bazel has written the log, and an earlier
            // run's is gone before then, so a build that never writes it
            // leaves nothing to be read as its own.
            clear_stale(&copies);
            if !followed.appear() {
                if let Some(Done::Broken { error, .. }) = followed.take_done() {
                    return Err(error.into());
                }
                sender.close()?;
                return Ok(());
            }
            let mut file = MultiTeeReader {
                inner: followed,
                tees: copies
                    .into_iter()
                    .map(|path| Tee::open("WorkspaceLog", path))
                    .collect(),
            };
            let mut buf: Vec<u8> = Vec::with_capacity(1024 * 5);
            let mut has_readers = true;
            let mut undecodable = None;
            let read = (|| -> Result<(), WorkspaceEventError> {
                loop {
                    match delimited::next_frame(&mut file, &mut buf) {
                        Ok(Some(header)) => {
                            if has_readers {
                                match WorkspaceEvent::decode(&buf[..header.len]) {
                                    Ok(event) => has_readers = sender.send(event).is_ok(),
                                    Err(err) => {
                                        has_readers = false;
                                        sender.close()?;
                                        undecodable = Some(err);
                                    }
                                }
                            }
                        }
                        // The end of the stream: between two events, or cut off
                        // in the middle of one. Why it ended is the file's to say.
                        Ok(None) => break,
                        Err(err) if StartedOver::is(&err) => file.restart(),
                        Err(err) if err.kind() == ErrorKind::UnexpectedEof => break,
                        Err(err) => return Err(err.into()),
                    }
                }
                if let Some(Done::Broken { error, .. }) = file.inner.take_done() {
                    return Err(error.into());
                }
                if has_readers {
                    sender.close()?;
                }
                let what = format!("copying '{}' again", file.inner.path().display());
                let source = file.inner.rewound();
                file.recopy(source, &what);
                match undecodable {
                    Some(err) => Err(err.into()),
                    None => Ok(()),
                }
            })();
            // Every copy is finished however reading ended; a reader error is
            // the one reported when both fail.
            let copies = file
                .finish()
                .map_err(|e| WorkspaceEventError::Copy(e.to_string()));
            read.and(copies)
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
        let log = LogFile::reserve();
        let path = log.path().to_owned();
        let mut gate = PidPair {
            client: client.id(),
            server: None,
        }
        .gate();
        gate.quiet = Duration::from_millis(300);
        gate.quiet_empty = Duration::from_millis(100);
        (path, WorkspaceEventStream::spawn(log, gate, vec![]))
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

    /// A kept log stays once read, and each copy gets the whole log.
    #[test]
    fn a_kept_log_stays_and_its_copies_are_whole() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("workspace_events.binpb");
        let copy = dir.path().join("asked.log");
        let (read_path, copy_path) = (path.clone(), copy.clone());
        let outcome = crate::testing::with_timeout(Duration::from_secs(10), move || {
            let client = spawn_pid_holder();
            let mut gate = PidPair {
                client: client.id(),
                server: None,
            }
            .gate();
            gate.quiet = Duration::from_millis(300);
            let mut stream = WorkspaceEventStream::spawn(
                LogFile::Kept(read_path.clone()),
                gate,
                vec![copy_path.to_str().unwrap().to_owned()],
            );
            let recv = stream.receiver().unwrap();
            stream.seal();
            std::fs::write(&read_path, encoded(1..=2)).unwrap();
            end(client);
            let got = std::iter::from_fn(|| recv.recv().ok()).count();
            (got, stream.join().is_ok())
        });
        assert_eq!(outcome, Some((2, true)));
        assert_eq!(std::fs::read(&path).unwrap(), encoded(1..=2));
        assert_eq!(std::fs::read(&copy).unwrap(), encoded(1..=2));
    }

    /// Bazel starts its log over in place for a retried attempt: both attempts
    /// reach the subscriber, and a copy holds only the last, as the kept file
    /// does.
    #[test]
    fn a_log_started_over_in_place_starts_its_copies_over() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("workspace_events.binpb");
        let copy = dir.path().join("asked.log");
        let (read_path, copy_path) = (path.clone(), copy.clone());
        let got = crate::testing::with_timeout(Duration::from_secs(10), move || {
            let client = spawn_pid_holder();
            let mut gate = PidPair {
                client: client.id(),
                server: None,
            }
            .gate();
            gate.quiet = Duration::from_millis(300);
            std::fs::write(&read_path, encoded(1..=50)).unwrap();
            let mut stream = WorkspaceEventStream::spawn(
                LogFile::Kept(read_path.clone()),
                gate,
                vec![copy_path.to_str().unwrap().to_owned()],
            );
            let recv = stream.receiver().unwrap();
            stream.seal();
            let mut got: Vec<String> = (0..50).map(|_| recv.recv().unwrap().location).collect();
            // Shorter than what was read, so the follower sees it truncated.
            std::fs::write(&read_path, encoded([100])).unwrap();
            got.push(recv.recv().unwrap().location);
            end(client);
            assert!(recv.recv().is_err(), "the stream ends with the client");
            stream.join().unwrap();
            got
        })
        .expect("the stream hung");
        assert_eq!(got.len(), 51);
        assert_eq!(got.last().unwrap(), "100");
        assert_eq!(std::fs::read(&copy).unwrap(), encoded([100]));
    }

    /// Follows a kept log with one copy, writing `first`, then after the reader
    /// has had time to read it, `then` in its place. Returns the copy and how
    /// the stream joined.
    fn copy_after(first: Vec<u8>, then: Vec<u8>) -> (Vec<u8>, Result<(), WorkspaceEventError>) {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("workspace_events.binpb");
        let copy = dir.path().join("asked.log");
        let (read_path, copy_path) = (path.clone(), copy.clone());
        let joined = crate::testing::with_timeout(Duration::from_secs(10), move || {
            let client = spawn_pid_holder();
            let mut gate = PidPair {
                client: client.id(),
                server: None,
            }
            .gate();
            gate.quiet = Duration::from_millis(300);
            std::fs::write(&read_path, first).unwrap();
            let mut stream = WorkspaceEventStream::spawn(
                LogFile::Kept(read_path.clone()),
                gate,
                vec![copy_path.to_str().unwrap().to_owned()],
            );
            stream.seal();
            std::thread::sleep(Duration::from_millis(500));
            std::fs::write(&read_path, then).unwrap();
            end(client);
            stream.join()
        })
        .expect("the stream hung");
        (std::fs::read(&copy).unwrap(), joined)
    }

    /// A restart the reader cannot see, because the new attempt is already
    /// longer than what it read, still leaves the copy as Bazel's final file
    /// rather than the old attempt with the new one's tail.
    #[test]
    fn a_missed_restart_leaves_the_copy_as_bazels_file() {
        let (copy, joined) = copy_after(encoded(1..=20), encoded(100..=200));
        assert!(joined.is_ok(), "{joined:?}");
        assert_eq!(copy, encoded(100..=200));
    }

    /// An event that does not decode fails the stream, and the copy is still
    /// the whole file.
    #[test]
    fn an_undecodable_log_is_still_copied_whole() {
        let mut bytes = vec![4, 0xff, 0xff, 0xff, 0xff];
        bytes.extend(encoded(2..=3));
        let (copy, joined) = copy_after(bytes.clone(), bytes.clone());
        assert!(
            matches!(joined, Err(WorkspaceEventError::ProstDecode(_))),
            "{joined:?}"
        );
        assert_eq!(copy, bytes);
    }

    /// A build that never writes the log leaves no earlier run's copy behind.
    #[test]
    fn a_log_that_never_appears_leaves_no_stale_copy() {
        let dir = tempfile::tempdir().unwrap();
        let copy = dir.path().join("asked.log");
        std::fs::write(&copy, b"an earlier run's").unwrap();
        let copy_path = copy.to_str().unwrap().to_owned();
        let joined = crate::testing::with_timeout(Duration::from_secs(10), move || {
            let client = spawn_pid_holder();
            let mut gate = PidPair {
                client: client.id(),
                server: None,
            }
            .gate();
            gate.quiet_empty = Duration::from_millis(100);
            let stream = WorkspaceEventStream::spawn(LogFile::reserve(), gate, vec![copy_path]);
            end(client);
            stream.join().is_ok()
        });
        assert_eq!(joined, Some(true));
        assert!(!copy.exists(), "an earlier run's copy outlived the build");
    }

    /// A copy that cannot be created fails the join, but the subscriber still
    /// gets every event.
    #[test]
    fn a_copy_that_cannot_open_fails_alone() {
        let dir = tempfile::tempdir().unwrap();
        let bad = dir.path().join("missing-dir").join("asked.log");
        let bad_path = bad.to_str().unwrap().to_owned();
        let outcome = crate::testing::with_timeout(Duration::from_secs(10), move || {
            let client = spawn_pid_holder();
            let log = LogFile::reserve();
            let path = log.path().to_owned();
            let mut gate = PidPair {
                client: client.id(),
                server: None,
            }
            .gate();
            gate.quiet = Duration::from_millis(300);
            let mut stream = WorkspaceEventStream::spawn(log, gate, vec![bad_path]);
            let recv = stream.receiver().unwrap();
            stream.seal();
            std::fs::write(&path, encoded(1..=3)).unwrap();
            end(client);
            let got = std::iter::from_fn(|| recv.recv().ok()).count();
            (got, stream.join().map_err(|e| e.to_string()))
        })
        .unwrap();
        assert_eq!(outcome.0, 3);
        let err = outcome.1.unwrap_err();
        assert!(
            err.contains(bad.to_str().unwrap()) && err.contains("failed to create"),
            "{err}"
        );
    }
}
