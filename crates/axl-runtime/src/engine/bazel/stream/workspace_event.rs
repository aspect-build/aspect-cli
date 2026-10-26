use axl_proto::workspace_log::WorkspaceEvent;
use fibre::spmc::{Receiver, bounded};
use fibre::{CloseError, SendError};
use prost::Message;
use std::fmt::Debug;
use std::io::{self, ErrorKind, Read};
use std::path::PathBuf;
use std::sync::atomic::Ordering;
use std::thread::JoinHandle;
use std::{env, thread};
use thiserror::Error;

use super::build_event::{PendingWriterReader, spawn_open_watchdog};
use super::util::read_varint;

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
    /// Mint a temp FIFO path and create the inode, returning the path so the
    /// caller can pass it to bazel before spawning. Pair with `spawn` once the
    /// client's pid is known.
    pub fn reserve_path() -> io::Result<PathBuf> {
        let out = env::temp_dir().join(format!("workspace-event-out-{}.bin", uuid::Uuid::new_v4()));
        galvanize::Pipe::mkfifo(&out)?;
        Ok(out)
    }

    /// Spawn the reader. `server_pid` is the bazel server that holds the FIFO
    /// open while it writes; `writer_pid` is the client of this invocation,
    /// whose death ends the stream even when the server still holds the pipe
    /// or never opened it (see `BuildEventStream::spawn`).
    pub fn spawn(path: PathBuf, server_pid: u32, writer_pid: u32) -> io::Result<Self> {
        let (mut sender, recv) = bounded::<WorkspaceEvent>(1000);
        let handle: JoinHandle<Result<(), WorkspaceEventError>> = thread::spawn(move || {
            let mut buf: Vec<u8> = Vec::with_capacity(1024 * 5);
            // 10 is the maximum size of a varint so start with that size.
            buf.resize(10, 0);

            galvanize::Pipe::mkfifo(&path)?;
            let opened = spawn_open_watchdog(path.clone(), writer_pid);
            let inner =
                galvanize::Pipe::open(path, galvanize::RetryPolicy::IfOpenForPid(server_pid))?;
            inner.set_nonblocking()?;
            opened.store(true, Ordering::Relaxed);
            let mut out_raw = PendingWriterReader { inner, writer_pid };

            let mut read = || -> Result<(), WorkspaceEventError> {
                // varint size can be somewhere between 1 to 10 bytes.
                let (size, _) = read_varint(&mut out_raw)?;
                if size > buf.len() {
                    buf.resize(size, 0);
                }

                out_raw.read_exact(&mut buf[0..size])?;

                let event = WorkspaceEvent::decode(&buf[0..size])?;

                // Send blocks until there is room in the buffer.
                // https://docs.rs/fibre/latest/fibre/spmc/index.html
                sender.send(event)?;

                Ok(())
            };

            loop {
                let result = read();

                // event decoding was succesfull move to the next.
                if result.is_ok() {
                    continue;
                }

                match result.unwrap_err() {
                    // this marks the end of the stream
                    WorkspaceEventError::IO(err) if err.kind() == ErrorKind::BrokenPipe => {
                        sender.close()?;
                        return Ok(());
                    }
                    err => return Err(err),
                }
            }
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
    use std::io::Write;
    use std::time::Duration;

    fn temp_fifo_path() -> PathBuf {
        env::temp_dir().join(format!("test-workspace-{}.fifo", uuid::Uuid::new_v4()))
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

    /// Bazel can exit before it opens the workspace log, leaving a live server
    /// that looks like a writer on its way.
    #[test]
    fn a_client_that_never_opens_the_log_ends_the_stream() {
        let joined = crate::test::with_timeout(Duration::from_secs(10), || {
            let mut server = spawn_pid_holder();
            let mut client = spawn_pid_holder();
            let path = temp_fifo_path();
            galvanize::Pipe::mkfifo(&path).unwrap();
            let stream = WorkspaceEventStream::spawn(path, server.id(), client.id()).unwrap();
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
        let outcome = crate::test::with_timeout(Duration::from_secs(10), || {
            let mut server = spawn_pid_holder();
            let mut client = spawn_pid_holder();
            let path = temp_fifo_path();
            galvanize::Pipe::mkfifo(&path).unwrap();
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
            let stream = WorkspaceEventStream::spawn(path, server.id(), client.id()).unwrap();
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
}
