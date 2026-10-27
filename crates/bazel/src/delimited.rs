//! Length-delimited framing: a varint length, then that many bytes. What
//! protobuf's `encode_length_delimited` writes and bazel's binary logs use.

use std::io::{self, ErrorKind, Read};

/// The largest frame accepted. A length this big is a corrupt stream, not an
/// event: without a bound it would be allocated as asked.
pub const MAX_FRAME: usize = 512 * 1024 * 1024;

const CONTINUATION_BIT: u8 = 1 << 7;

/// The length prefix of one frame: its raw bytes (kept so a mirror of the
/// stream can write the frame back out unchanged) and the length they encode.
#[derive(Debug, PartialEq, Eq)]
pub struct Header {
    pub prefix: Vec<u8>,
    pub len: usize,
}

/// Read a varint, returning its value and the raw bytes it was encoded in.
pub fn read_varint<R: Read>(stream: &mut R) -> io::Result<(usize, Vec<u8>)> {
    let mut first = [0];
    stream.read_exact(&mut first)?;
    varint_after(first[0], stream)
}

/// The rest of a varint whose first byte has been read.
fn varint_after<R: Read>(first: u8, stream: &mut R) -> io::Result<(usize, Vec<u8>)> {
    let mut result: u64 = 0;
    let mut shift = 0;
    let mut raw = Vec::new();
    let mut byte = first;

    loop {
        raw.push(byte);

        if shift == 63 && byte != 0x00 && byte != 0x01 {
            // Consume the rest of the varint so the stream stays aligned on
            // the next one, then refuse this one.
            while byte & CONTINUATION_BIT != 0 {
                let mut next = [0];
                stream.read_exact(&mut next)?;
                byte = next[0];
                raw.push(byte);
            }
            return Err(io::Error::new(ErrorKind::InvalidData, "varint overflow"));
        }

        result |= ((byte & !CONTINUATION_BIT) as u64) << shift;

        if byte & CONTINUATION_BIT == 0 {
            return Ok((result as usize, raw));
        }

        shift += 7;
        let mut next = [0];
        stream.read_exact(&mut next)?;
        byte = next[0];
    }
}

/// Read one frame: its header, with the payload left in `buf[..header.len]`
/// (`buf` grows as needed and is reused across calls).
///
/// `Ok(None)` is a clean end: the stream ended exactly between two frames.
/// A stream that ends inside a frame is `UnexpectedEof`, and a length over
/// [`MAX_FRAME`] is `InvalidData`.
pub fn next_frame<R: Read>(stream: &mut R, buf: &mut Vec<u8>) -> io::Result<Option<Header>> {
    let first = loop {
        let mut byte = [0];
        match stream.read(&mut byte) {
            Ok(0) => return Ok(None),
            Ok(_) => break byte[0],
            Err(e) if e.kind() == ErrorKind::Interrupted => continue,
            Err(e) => return Err(e),
        }
    };
    let (len, prefix) = varint_after(first, stream)?;
    if len > MAX_FRAME {
        return Err(io::Error::new(
            ErrorKind::InvalidData,
            format!("frame of {len} bytes exceeds the {MAX_FRAME} byte limit"),
        ));
    }
    if len > buf.len() {
        buf.resize(len, 0);
    }
    stream.read_exact(&mut buf[..len])?;
    Ok(Some(Header { prefix, len }))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Cursor;

    fn varint(mut v: u64) -> Vec<u8> {
        let mut out = Vec::new();
        loop {
            let low = (v & 0x7f) as u8;
            v >>= 7;
            if v == 0 {
                out.push(low);
                return out;
            }
            out.push(low | CONTINUATION_BIT);
        }
    }

    fn frame(payload: &[u8]) -> Vec<u8> {
        let mut out = varint(payload.len() as u64);
        out.extend_from_slice(payload);
        out
    }

    /// Hands out one byte per read, as a pipe fed in tiny pieces does.
    struct Trickle<R>(R);

    impl<R: Read> Read for Trickle<R> {
        fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
            let n = buf.len().min(1);
            self.0.read(&mut buf[..n])
        }
    }

    #[test]
    fn a_one_byte_varint_reads_back() {
        let (v, raw) = read_varint(&mut Cursor::new(vec![0x05])).unwrap();
        assert_eq!((v, raw), (5, vec![0x05]));
    }

    #[test]
    fn a_multi_byte_varint_reads_back_with_its_raw_bytes() {
        let bytes = varint(300);
        assert_eq!(bytes.len(), 2);
        let (v, raw) = read_varint(&mut Cursor::new(bytes.clone())).unwrap();
        assert_eq!((v, raw), (300, bytes));
    }

    #[test]
    fn the_largest_varint_reads_back() {
        let bytes = varint(u64::MAX);
        assert_eq!(bytes.len(), 10);
        let (v, raw) = read_varint(&mut Cursor::new(bytes.clone())).unwrap();
        assert_eq!(v, u64::MAX as usize);
        assert_eq!(raw, bytes);
    }

    #[test]
    fn an_overflowing_varint_is_refused_and_fully_consumed() {
        // Eleven bytes: the tenth still has its continuation bit set.
        let mut bytes = vec![0xff; 10];
        bytes.push(0x01);
        bytes.push(0x2a); // the next stream byte, which must be left alone
        let mut cursor = Cursor::new(bytes);
        let err = read_varint(&mut cursor).unwrap_err();
        assert_eq!(err.kind(), ErrorKind::InvalidData);
        assert_eq!(cursor.position(), 11, "the whole bad varint was consumed");
    }

    #[test]
    fn a_varint_cut_short_is_unexpected_eof() {
        let err = read_varint(&mut Cursor::new(vec![0x80])).unwrap_err();
        assert_eq!(err.kind(), ErrorKind::UnexpectedEof);
        let err = read_varint(&mut Cursor::new(Vec::new())).unwrap_err();
        assert_eq!(err.kind(), ErrorKind::UnexpectedEof);
    }

    #[test]
    fn frames_read_back_in_order_with_their_prefixes() {
        let mut bytes = frame(b"one");
        bytes.extend(frame(b"second"));
        bytes.extend(frame(&[7u8; 200])); // a two-byte prefix
        let mut cursor = Cursor::new(bytes);
        let mut buf = Vec::new();

        let h = next_frame(&mut cursor, &mut buf).unwrap().unwrap();
        assert_eq!((h.len, h.prefix.clone()), (3, vec![3]));
        assert_eq!(&buf[..h.len], b"one");

        let h = next_frame(&mut cursor, &mut buf).unwrap().unwrap();
        assert_eq!(h.len, 6);
        assert_eq!(&buf[..h.len], b"second");

        let h = next_frame(&mut cursor, &mut buf).unwrap().unwrap();
        assert_eq!((h.len, h.prefix.len()), (200, 2));
        assert_eq!(&buf[..h.len], &[7u8; 200][..]);

        assert_eq!(next_frame(&mut cursor, &mut buf).unwrap(), None);
    }

    #[test]
    fn an_empty_frame_is_a_frame_not_the_end() {
        let mut cursor = Cursor::new(frame(b""));
        let mut buf = Vec::new();
        let h = next_frame(&mut cursor, &mut buf).unwrap().unwrap();
        assert_eq!((h.len, h.prefix), (0, vec![0]));
        assert_eq!(next_frame(&mut cursor, &mut buf).unwrap(), None);
    }

    #[test]
    fn an_empty_stream_is_a_clean_end() {
        let mut buf = Vec::new();
        assert_eq!(
            next_frame(&mut Cursor::new(Vec::new()), &mut buf).unwrap(),
            None
        );
    }

    #[test]
    fn a_stream_cut_inside_a_prefix_is_unexpected_eof() {
        let mut buf = Vec::new();
        let err = next_frame(&mut Cursor::new(vec![0x80]), &mut buf).unwrap_err();
        assert_eq!(err.kind(), ErrorKind::UnexpectedEof);
    }

    #[test]
    fn a_stream_cut_inside_a_payload_is_unexpected_eof() {
        let mut bytes = frame(b"complete");
        bytes.truncate(bytes.len() - 3);
        let mut buf = Vec::new();
        let err = next_frame(&mut Cursor::new(bytes), &mut buf).unwrap_err();
        assert_eq!(err.kind(), ErrorKind::UnexpectedEof);
    }

    #[test]
    fn a_frame_over_the_limit_is_refused_before_it_is_allocated() {
        let bytes = varint(MAX_FRAME as u64 + 1);
        let mut buf = Vec::new();
        let err = next_frame(&mut Cursor::new(bytes), &mut buf).unwrap_err();
        assert_eq!(err.kind(), ErrorKind::InvalidData);
        assert!(buf.is_empty(), "nothing was allocated for the claimed size");
    }

    #[test]
    fn a_frame_at_the_limit_is_not_refused_for_its_size() {
        // Only the prefix is supplied, so it fails reading the payload, not
        // on the size check.
        let bytes = varint(MAX_FRAME as u64);
        let mut buf = Vec::new();
        // Not allocating 512 MiB here: a zero-length source makes
        // `read_exact` fail right after the size check passes.
        let err = next_frame(&mut Cursor::new(bytes), &mut buf).unwrap_err();
        assert_eq!(err.kind(), ErrorKind::UnexpectedEof);
        buf.clear();
    }

    #[test]
    fn the_buffer_is_reused_and_only_grows() {
        let mut bytes = frame(&[1u8; 100]);
        bytes.extend(frame(b"tiny"));
        let mut cursor = Cursor::new(bytes);
        let mut buf = Vec::new();
        next_frame(&mut cursor, &mut buf).unwrap().unwrap();
        let grown = buf.len();
        let h = next_frame(&mut cursor, &mut buf).unwrap().unwrap();
        assert_eq!(buf.len(), grown, "a smaller frame does not shrink it");
        assert_eq!(&buf[..h.len], b"tiny");
    }

    #[test]
    fn frames_survive_a_source_that_trickles_one_byte_at_a_time() {
        let mut bytes = frame(&[9u8; 300]);
        bytes.extend(frame(b"after"));
        let mut src = Trickle(Cursor::new(bytes));
        let mut buf = Vec::new();
        let h = next_frame(&mut src, &mut buf).unwrap().unwrap();
        assert_eq!((h.len, h.prefix.len()), (300, 2));
        let h = next_frame(&mut src, &mut buf).unwrap().unwrap();
        assert_eq!(&buf[..h.len], b"after");
        assert_eq!(next_frame(&mut src, &mut buf).unwrap(), None);
    }

    #[test]
    fn an_interrupted_first_read_is_retried() {
        struct Flaky {
            interrupted: bool,
            inner: Cursor<Vec<u8>>,
        }
        impl Read for Flaky {
            fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
                if !self.interrupted {
                    self.interrupted = true;
                    return Err(io::Error::from(ErrorKind::Interrupted));
                }
                self.inner.read(buf)
            }
        }
        let mut src = Flaky {
            interrupted: false,
            inner: Cursor::new(frame(b"ok")),
        };
        let mut buf = Vec::new();
        let h = next_frame(&mut src, &mut buf).unwrap().unwrap();
        assert_eq!(&buf[..h.len], b"ok");
    }

    #[test]
    fn a_read_error_on_the_first_byte_is_the_error_not_an_end() {
        struct Broken;
        impl Read for Broken {
            fn read(&mut self, _: &mut [u8]) -> io::Result<usize> {
                Err(io::Error::from(ErrorKind::PermissionDenied))
            }
        }
        let mut buf = Vec::new();
        let err = next_frame(&mut Broken, &mut buf).unwrap_err();
        assert_eq!(err.kind(), ErrorKind::PermissionDenied);
    }
}
