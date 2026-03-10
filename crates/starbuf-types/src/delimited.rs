use std::fmt::Debug;
use std::io::{Cursor, Read};
use std::sync::Mutex;

use allocative::Allocative;
use starlark::starlark_simple_value;
use starlark::typing::Ty;
use starlark::values;
use starlark::values::Heap;
use starlark::values::NoSerialize;
use starlark::values::ProvidesStaticType;
use starlark::values::Trace;
use starlark::values::starlark_value;

use crate::any_registry::AnyAllocatable;

/// Source of bytes for the delimited iterator.
pub enum ReadSource {
    Bytes(Cursor<Vec<u8>>),
    Stream(Box<dyn Read + Send>),
}

impl Read for ReadSource {
    fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
        match self {
            ReadSource::Bytes(cursor) => cursor.read(buf),
            ReadSource::Stream(reader) => reader.read(buf),
        }
    }
}

/// Read a varint from the reader. Returns `None` on clean EOF (first byte).
fn read_varint(reader: &mut dyn Read) -> std::io::Result<Option<u64>> {
    let mut result: u64 = 0;
    let mut shift = 0u32;
    let mut buf = [0u8; 1];
    loop {
        match reader.read_exact(&mut buf) {
            Ok(()) => {}
            Err(e) if e.kind() == std::io::ErrorKind::UnexpectedEof && shift == 0 => {
                return Ok(None);
            }
            Err(e) => return Err(e),
        }
        let byte = buf[0];
        result |= ((byte & 0x7F) as u64) << shift;
        if byte & 0x80 == 0 {
            return Ok(Some(result));
        }
        shift += 7;
        if shift >= 64 {
            return Err(std::io::Error::new(
                std::io::ErrorKind::InvalidData,
                "varint too large",
            ));
        }
    }
}

type DecoderFn = dyn Fn(&[u8]) -> anyhow::Result<Box<dyn AnyAllocatable>> + Send + Sync;

/// A lazy iterator that parses length-delimited protobuf messages on demand.
#[derive(ProvidesStaticType, Trace, NoSerialize, Allocative)]
pub struct DelimitedMessageIterator {
    #[allocative(skip)]
    reader: Mutex<ReadSource>,
    #[allocative(skip)]
    decoder: Box<DecoderFn>,
}

impl Debug for DelimitedMessageIterator {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("DelimitedMessageIterator").finish()
    }
}

impl std::fmt::Display for DelimitedMessageIterator {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "<DelimitedMessageIterator>")
    }
}

impl DelimitedMessageIterator {
    pub fn new(source: ReadSource, decoder: Box<DecoderFn>) -> Self {
        Self {
            reader: Mutex::new(source),
            decoder,
        }
    }
}

#[starlark_value(type = "DelimitedMessageIterator")]
impl<'v> values::StarlarkValue<'v> for DelimitedMessageIterator {
    fn get_type_starlark_repr() -> Ty {
        Ty::iter(Ty::any())
    }

    unsafe fn iterate(
        &self,
        me: values::Value<'v>,
        _heap: Heap<'v>,
    ) -> starlark::Result<values::Value<'v>> {
        // The iterator IS the iterable — return self.
        Ok(me)
    }

    unsafe fn iter_next(&self, _index: usize, heap: Heap<'v>) -> Option<values::Value<'v>> {
        let mut reader = self.reader.lock().unwrap();
        let len = match read_varint(&mut *reader) {
            Ok(Some(len)) => len as usize,
            Ok(None) => return None, // clean EOF
            Err(_) => return None,
        };
        let mut buf = vec![0u8; len];
        if reader.read_exact(&mut buf).is_err() {
            return None;
        }
        match (self.decoder)(&buf) {
            Ok(value) => Some(value.alloc_on(heap)),
            Err(_) => None,
        }
    }

    unsafe fn iter_stop(&self) {}
}

starlark_simple_value!(DelimitedMessageIterator);
