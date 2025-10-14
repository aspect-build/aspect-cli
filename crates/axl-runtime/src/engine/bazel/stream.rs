use axl_proto::build_event_stream::BuildEvent;
use fibre::spmc::{bounded, Receiver};
use fibre::SendError;
use prost::Message;
use std::{
    fs::File,
    io::Read,
    path::PathBuf,
    thread::{self, JoinHandle},
};
use thiserror::Error;

use super::stream_util::read_varint;

#[derive(Error, Debug)]
pub enum StreamError {
    #[error("io error: {0}")]
    IoError(#[from] std::io::Error),
    #[error("prost decode error: {0}")]
    ProstDecodeError(#[from] prost::DecodeError),
    #[error("send error: {0}")]
    SendError(#[from] SendError),
}

#[derive(Debug)]
pub struct EventStream {
    handle: JoinHandle<Result<(), StreamError>>,
    recv: Receiver<BuildEvent>,
}

impl EventStream {
    pub fn spawn(path: PathBuf) -> Self {
        let (sender, recv) = bounded::<BuildEvent>(1000);
        let handle = thread::spawn(move || {
            let mut buf: Vec<u8> = Vec::with_capacity(1024 * 5);
            // 10 is the maximum size of a varint so start with that size.
            buf.resize(10, 0);

            while !path.exists() {
                // TODO: some debug logging, so that this condition is visible.
            }

            let mut out_raw = File::open(&path)?;

            loop {
                // varint size can be somewhere between 1 to 10 bytes.
                let size = read_varint(&mut out_raw)?;

                if size > buf.len() {
                    buf.resize(size, 0);
                }

                out_raw.read_exact(&mut buf[0..size])?;

                let event = BuildEvent::decode(&buf[0..size])?;
                let last_message = event.last_message;

                sender.send(event)?;

                if last_message {
                    drop(sender);
                    break;
                }
            }

            return Ok(());
        });
        Self { handle, recv }
    }

    pub fn receiver(&self) -> Receiver<BuildEvent> {
        self.recv.clone()
    }

    pub fn join(self) -> Result<(), StreamError> {
        self.handle.join().expect("join error")
    }
}
