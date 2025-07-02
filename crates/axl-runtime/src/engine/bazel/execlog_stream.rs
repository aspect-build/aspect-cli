use prost::Message;
use std::io::ErrorKind;
use std::io::Read;
use std::panic;
use std::time::Duration;
use thiserror::Error;

pub struct RetryStream<T: Read> {
    pub(super) inner: T,
}

impl<T: Read> Read for RetryStream<T> {
    fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
        let mut tries = 0;
        loop {
            match self.inner.read(buf) {
                Ok(n) => return Ok(n),
                Err(err) if err.kind() == ErrorKind::UnexpectedEof => {
                    tries += 1;
                    if tries > 10 {
                        return Err(std::io::Error::new(err.kind(), err));
                    }
                    std::thread::sleep(Duration::from_millis(500));
                    continue;
                }
                Err(err) => return Err(err),
            }
        }
    }
}

#[derive(Error, Debug)]
pub enum Error {
    #[error("io error: {0}")]
    IoError(#[from] std::io::Error),
    #[error("prost decode error: {0}")]
    ProstDecodeError(#[from] prost::DecodeError),
}

pub type Result<T> = std::result::Result<T, Error>;

#[derive(Debug, Clone)]
pub struct ExecLogIterator<T> {
    stream: T,
    buf: Vec<u8>,
}

impl<T> Iterator for ExecLogIterator<T>
where
    T: Read,
{
    type Item = axl_proto::tools::protos::ExecLogEntry;

    fn next(&mut self) -> Option<Self::Item> {
        match self.next_entry::<Self::Item>() {
            Ok(v) => Some(v),
            // TODO: handle the error, if its fatal panic
            Err(_err) => None,
        }
    }
}

pub const CONTINUATION_BIT: u8 = 1 << 7;

#[inline]
pub fn low_bits_of_byte(byte: u8) -> u8 {
    byte & !CONTINUATION_BIT
}

impl<T: Read> ExecLogIterator<T> {
    pub fn new(stream: T) -> Self {
        let mut buf = Vec::with_capacity(1024 * 5);
        buf.resize(10, 0);
        Self { stream, buf }
    }

    pub fn read_varint(&mut self) -> std::result::Result<usize, std::io::Error> {
        let mut result = 0;
        let mut shift = 0;

        loop {
            let mut buf = [0];
            self.stream.read_exact(&mut buf)?;

            if shift == 63 && buf[0] != 0x00 && buf[0] != 0x01 {
                while buf[0] & CONTINUATION_BIT != 0 {
                    self.stream.read_exact(&mut buf)?;
                }
                panic!("overflow");
            }

            let low_bits = low_bits_of_byte(buf[0]) as u64;
            result |= low_bits << shift;

            if buf[0] & CONTINUATION_BIT == 0 {
                return Ok(result as usize);
            }

            shift += 7;
        }
    }

    pub fn next_entry<M: Message + Default>(&mut self) -> Result<M> {
        // varint size can be somewhere between 1 to 10 bytes.
        let size = self.read_varint()?;

        if size > self.buf.len() {
            self.buf.resize(size, 0);
        }

        self.stream.read_exact(&mut self.buf[0..size])?;

        Ok(M::decode(&self.buf[0..size])?)
    }
}
