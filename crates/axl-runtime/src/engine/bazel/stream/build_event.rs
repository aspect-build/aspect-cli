use axl_proto::build_event_stream::BuildEvent;
use fibre::spmc::{bounded, Receiver};
use fibre::{CloseError, SendError};
use prost::Message;
use std::io::ErrorKind;
use std::{env, io};
use std::{
    io::Read,
    path::PathBuf,
    thread::{self, JoinHandle},
};
use thiserror::Error;

use super::util::read_varint;

#[derive(Error, Debug)]
pub enum BuildEventStreamError {
    #[error("io error: {0}")]
    IO(#[from] std::io::Error),
    #[error("prost decode error: {0}")]
    ProstDecode(#[from] prost::DecodeError),
    #[error("send error: {0}")]
    Send(#[from] SendError),
    #[error("close error: {0}")]
    Close(#[from] CloseError),
}

#[derive(Debug)]
pub struct BuildEventStream {
    handle: JoinHandle<Result<(), BuildEventStreamError>>,
    recv: Receiver<BuildEvent>,
}

impl BuildEventStream {
    pub fn spawn_with_pipe(pid: u32) -> io::Result<(PathBuf, Self)> {
        let out = env::temp_dir().join(format!("build-event-out-{}.bin", uuid::Uuid::new_v4()));
        let stream = Self::spawn(out.clone(), pid)?;
        Ok((out, stream))
    }

    pub fn spawn(path: PathBuf, pid: u32) -> io::Result<Self> {
        let (mut sender, recv) = bounded::<BuildEvent>(1000);
        let handle = thread::spawn(move || {
            let mut buf: Vec<u8> = Vec::with_capacity(1024 * 5);
            // 10 is the maximum size of a varint so start with that size.
            buf.resize(10, 0);
            let mut out_raw =
                galvanize::Pipe::new(path.clone(), galvanize::RetryPolicy::IfOpenForPid(pid))?;
            let mut read = || -> Result<bool, BuildEventStreamError> {
                // varint size can be somewhere between 1 to 10 bytes.
                let size = read_varint(&mut out_raw)?;
                if size > buf.len() {
                    buf.resize(size, 0);
                }

                out_raw.read_exact(&mut buf[0..size])?;

                let event = BuildEvent::decode(&buf[0..size])?;
                let last_message = event.last_message;

                // Send blocks until there is room in the buffer.
                // https://docs.rs/fibre/latest/fibre/spmc/index.html
                sender.send(event)?;

                Ok(last_message)
            };

            loop {
                match read() {
                    // marks the end of the stream
                    Ok(last_message) if last_message => {
                        sender.close()?;
                        return Ok(());
                    }
                    // marks the end of the stream
                    Err(BuildEventStreamError::IO(err)) if err.kind() == ErrorKind::BrokenPipe => {
                        sender.close()?;
                        return Ok(());
                    }
                    Ok(_) => continue,
                    Err(err) => return Err(err),
                }
            }
        });
        Ok(Self { handle, recv })
    }

    pub fn receiver(&self) -> Receiver<BuildEvent> {
        self.recv.clone()
    }

    pub fn join(self) -> Result<(), BuildEventStreamError> {
        self.handle.join().expect("join error")
    }
}
