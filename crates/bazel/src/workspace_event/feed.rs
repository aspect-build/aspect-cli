use axl_proto::workspace_log::WorkspaceEvent;
use fibre::spmc::{Receiver, bounded};
use fibre::{CloseError, SendError};
use prost::Message;
use std::fmt::Debug;
use std::io::{self, ErrorKind};
use std::thread;
use std::thread::JoinHandle;
use thiserror::Error;

use crate::delimited;
use crate::stream::Gate;
use crate::stream::pipe::Channel;

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
    recv: Receiver<WorkspaceEvent>,
}

impl Debug for WorkspaceEventStream {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("WorkspaceEventStream")
            .field("stream", &String::from("hidden"))
            .finish()
    }
}

impl WorkspaceEventStream {
    /// Reserve the FIFO the workspace log is written to, so its path can go on
    /// the command line before bazel is spawned. Pair with
    /// [`spawn`](Self::spawn) once the bazel client exists.
    pub fn reserve() -> io::Result<Channel> {
        Channel::fifo("workspace-event-out")
    }

    /// Spawn the reader on `channel`. `gate` decides when the stream is over
    /// (see [`crate::stream`]); for a real bazel it comes from
    /// `PidPair::gate`. The workspace log has no end marker, so the stream
    /// ends only through it.
    pub fn spawn(channel: Channel, gate: Gate) -> io::Result<Self> {
        let (mut sender, recv) = bounded::<WorkspaceEvent>(1000);
        let mut pipe = channel.follow(gate)?;
        let handle: JoinHandle<Result<(), WorkspaceEventError>> = thread::spawn(move || {
            let mut buf: Vec<u8> = Vec::with_capacity(1024 * 5);

            loop {
                match delimited::next_frame(&mut pipe, &mut buf) {
                    Ok(Some(header)) => {
                        let event = WorkspaceEvent::decode(&buf[..header.len])?;
                        // Send blocks until there is room in the buffer.
                        // https://docs.rs/fibre/latest/fibre/spmc/index.html
                        sender.send(event)?;
                    }
                    // The end of the stream: between two events, or cut off in
                    // the middle of one. Why it ended is the pipe's to say; the
                    // events before it were good either way.
                    Ok(None) => break,
                    Err(err) if err.kind() == ErrorKind::UnexpectedEof => break,
                    Err(err) => return Err(err.into()),
                }
            }
            sender.close()?;
            Ok(())
        });

        Ok(Self { handle, recv })
    }

    pub fn receiver(&self) -> Receiver<WorkspaceEvent> {
        self.recv.clone()
    }

    pub fn join(self) -> Result<(), WorkspaceEventError> {
        self.handle.join().expect("join error")
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::stream::pid_pair::PidPair;
    use std::io::Write;
    use std::path::PathBuf;
    use std::time::Duration;

    fn temp_fifo_path() -> PathBuf {
        std::env::temp_dir().join(format!("test-workspace-{}.fifo", uuid::Uuid::new_v4()))
    }

    fn spawn_pid_holder() -> std::process::Child {
        std::process::Command::new("sleep")
            .arg("60")
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .spawn()
            .expect("failed to spawn sleep")
    }

    fn encoded_event() -> Vec<u8> {
        WorkspaceEvent::default().encode_length_delimited_to_vec()
    }

    /// A stream whose quiet windows are short enough for a test.
    fn spawn_at(path: PathBuf, server_pid: u32, client_pid: u32) -> WorkspaceEventStream {
        let channel = Channel::at(path.clone()).unwrap();
        let mut gate = PidPair {
            client: client_pid,
            server: Some(server_pid),
        }
        .gate(&path);
        gate.quiet = Duration::from_millis(300);
        gate.quiet_empty = Duration::from_millis(100);
        WorkspaceEventStream::spawn(channel, gate).unwrap()
    }

    /// Bazel can exit before it opens the workspace log, leaving a live server
    /// that looks like a writer on its way.
    #[test]
    fn a_client_that_never_opens_the_log_ends_the_stream() {
        let joined = crate::testing::with_timeout(Duration::from_secs(10), || {
            let mut server = spawn_pid_holder();
            let mut client = spawn_pid_holder();
            let stream = spawn_at(temp_fifo_path(), server.id(), client.id());
            let _ = client.kill();
            let _ = client.wait();
            let joined = stream.join().is_ok();
            let _ = server.kill();
            joined
        });
        assert_eq!(joined, Some(true));
    }

    /// The server keeps the write end open after the client is gone: no
    /// end-of-file ever arrives, so the client's death has to end the stream.
    #[test]
    fn a_dead_client_ends_the_stream_while_the_server_holds_the_pipe() {
        let outcome = crate::testing::with_timeout(Duration::from_secs(10), || {
            let mut server = spawn_pid_holder();
            let mut client = spawn_pid_holder();
            let path = temp_fifo_path();
            let channel = Channel::at(path.clone()).unwrap();
            let (release, held) = std::sync::mpsc::channel::<()>();
            let path_w = path.clone();
            let writer = thread::spawn(move || {
                let mut f = std::fs::OpenOptions::new()
                    .write(true)
                    .open(&path_w)
                    .unwrap();
                f.write_all(&encoded_event()).unwrap();
                let _ = held.recv();
            });
            let mut gate = PidPair {
                client: client.id(),
                server: Some(server.id()),
            }
            .gate(&path);
            gate.quiet = Duration::from_millis(300);
            let stream = WorkspaceEventStream::spawn(channel, gate).unwrap();
            let recv = stream.receiver();
            let first = recv.recv().is_ok();
            let _ = client.kill();
            let _ = client.wait();
            let joined = stream.join().is_ok();
            let _ = release.send(());
            let _ = writer.join();
            let _ = server.kill();
            (first, joined)
        });
        assert_eq!(outcome, Some((true, true)));
    }

    #[test]
    fn events_arrive_in_order_and_the_stream_ends_when_the_writer_is_done() {
        let outcome = crate::testing::with_timeout(Duration::from_secs(10), || {
            let mut holder = spawn_pid_holder();
            let path = temp_fifo_path();
            let stream = spawn_at(path.clone(), holder.id(), holder.id());
            let recv = stream.receiver();
            {
                let mut f = std::fs::OpenOptions::new().write(true).open(&path).unwrap();
                for _ in 0..3 {
                    f.write_all(&encoded_event()).unwrap();
                }
            }
            let joined = stream.join().is_ok();
            let got = std::iter::from_fn(|| recv.recv().ok()).count();
            let _ = holder.kill();
            (joined, got)
        });
        assert_eq!(outcome, Some((true, 3)));
    }

    #[test]
    fn a_stream_cut_off_inside_an_event_keeps_the_events_before_it() {
        let outcome = crate::testing::with_timeout(Duration::from_secs(10), || {
            let mut holder = spawn_pid_holder();
            let path = temp_fifo_path();
            let stream = spawn_at(path.clone(), holder.id(), holder.id());
            let recv = stream.receiver();
            {
                let mut f = std::fs::OpenOptions::new().write(true).open(&path).unwrap();
                f.write_all(&encoded_event()).unwrap();
                // A length that promises more than is written.
                f.write_all(&[5, 0, 0]).unwrap();
            }
            let joined = stream.join().is_ok();
            let got = std::iter::from_fn(|| recv.recv().ok()).count();
            let _ = holder.kill();
            (joined, got)
        });
        assert_eq!(outcome, Some((true, 1)));
    }

    #[test]
    fn an_event_that_does_not_decode_is_an_error() {
        let outcome = crate::testing::with_timeout(Duration::from_secs(10), || {
            let mut holder = spawn_pid_holder();
            let path = temp_fifo_path();
            let stream = spawn_at(path.clone(), holder.id(), holder.id());
            {
                let mut f = std::fs::OpenOptions::new().write(true).open(&path).unwrap();
                f.write_all(&[4, 0xff, 0xff, 0xff, 0xff]).unwrap();
            }
            let result = stream.join();
            let _ = holder.kill();
            matches!(result, Err(WorkspaceEventError::ProstDecode(_)))
        });
        assert_eq!(outcome, Some(true));
    }

    #[test]
    fn an_absurd_event_length_is_an_error() {
        let outcome = crate::testing::with_timeout(Duration::from_secs(10), || {
            let mut holder = spawn_pid_holder();
            let path = temp_fifo_path();
            let stream = spawn_at(path.clone(), holder.id(), holder.id());
            {
                let mut f = std::fs::OpenOptions::new().write(true).open(&path).unwrap();
                f.write_all(&[0x80, 0x80, 0x80, 0x80, 0x80, 0x20]).unwrap();
            }
            let result = stream.join();
            let _ = holder.kill();
            matches!(result, Err(WorkspaceEventError::IO(e)) if e.kind() == ErrorKind::InvalidData)
        });
        assert_eq!(outcome, Some(true));
    }

    #[test]
    fn a_reserved_workspace_channel_is_named_for_it() {
        let channel = WorkspaceEventStream::reserve().unwrap();
        let name = channel
            .path()
            .file_name()
            .unwrap()
            .to_string_lossy()
            .to_string();
        assert!(name.starts_with("workspace-event-out-"), "{name}");
    }
}
