use axl_proto::workspace_log::WorkspaceEvent;
use fibre::spmc::{bounded, Receiver};
use fibre::{CloseError, SendError};
use prost::Message;
use std::fmt::Debug;
use std::io::{self, ErrorKind, Read};
use std::path::PathBuf;
use std::thread::JoinHandle;
use std::{env, thread};
use thiserror::Error;

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
    pub fn spawn_with_pipe(pid: u32) -> io::Result<(PathBuf, Self)> {
        let out = env::temp_dir().join(format!("workspace-event-out-{}.bin", uuid::Uuid::new_v4()));
        let stream = Self::spawn(out.clone(), pid)?;
        Ok((out, stream))
    }

    pub fn spawn(path: PathBuf, pid: u32) -> io::Result<Self> {
        let (mut sender, recv) = bounded::<WorkspaceEvent>(1000);
        let handle: JoinHandle<Result<(), WorkspaceEventError>> = thread::spawn(move || {
            let mut buf: Vec<u8> = Vec::with_capacity(1024 * 5);
            // 10 is the maximum size of a varint so start with that size.
            buf.resize(10, 0);

            let mut out_raw =
                galvanize::Pipe::new(path.clone(), galvanize::RetryPolicy::IfOpenForPid(pid))?;

            let mut read = || -> Result<(), WorkspaceEventError> {
                // varint size can be somewhere between 1 to 10 bytes.
                let size = read_varint(&mut out_raw)?;
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
