use std::fs::File;
use std::io;
use std::io::Read;
use std::io::Result;
use std::io::{BufWriter, Write};

/// Wraps a `Read` source and tees every byte read to one or more `BufWriter<File>` sinks.
///
/// Used to intercept raw bytes from a stream before any further processing,
/// allowing file sinks to capture a copy without a second pass.
pub(super) struct MultiTeeReader<R: Read> {
    pub(super) inner: R,
    pub(super) writers: Vec<BufWriter<File>>,
}

impl<R: Read> Read for MultiTeeReader<R> {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        let n = self.inner.read(buf)?;
        for w in &mut self.writers {
            w.write_all(&buf[..n])?;
        }
        Ok(n)
    }
}

impl<R: Read> Write for MultiTeeReader<R> {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        for w in &mut self.writers {
            w.write_all(buf)?;
        }
        Ok(buf.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        for w in &mut self.writers {
            w.flush()?;
        }
        Ok(())
    }
}

pub const CONTINUATION_BIT: u8 = 1 << 7;

#[inline]
pub fn low_bits_of_byte(byte: u8) -> u8 {
    byte & !CONTINUATION_BIT
}

pub fn read_varint<T: Read>(stream: &mut T) -> Result<usize> {
    let mut result = 0;
    let mut shift = 0;

    loop {
        let mut buf = [0];
        stream.read_exact(&mut buf)?;

        if shift == 63 && buf[0] != 0x00 && buf[0] != 0x01 {
            while buf[0] & CONTINUATION_BIT != 0 {
                stream.read_exact(&mut buf)?;
            }
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                anyhow::anyhow!("variant overflow"),
            ));
        }

        let low_bits = low_bits_of_byte(buf[0]) as u64;
        result |= low_bits << shift;

        if buf[0] & CONTINUATION_BIT == 0 {
            return Ok(result as usize);
        }

        shift += 7;
    }
}
