use allocative::Allocative;

use derive_more::Display;
use starlark::starlark_simple_value;
use starlark::values::AllocValue;
use std::fmt::Debug;
use std::io::Read;

use starlark::typing::Ty;
use starlark::values;
use starlark::values::starlark_value;
use starlark::values::Heap;
use starlark::values::NoSerialize;
use starlark::values::ProvidesStaticType;
use starlark::values::Trace;

use super::stream;

#[derive(Debug, ProvidesStaticType, Display, Trace, NoSerialize, Allocative)]
#[display("<read_iter>")]
pub struct ReadIterable {
    #[allocative(skip)]
    readable: stream::Readable,
}

impl ReadIterable {
    pub fn new(readable: stream::Readable) -> Self {
        Self { readable }
    }
}

#[starlark_value(type = "readable_iter")]
impl<'v> values::StarlarkValue<'v> for ReadIterable {
    fn get_type_starlark_repr() -> Ty {
        Ty::iter(Ty::string())
    }

    unsafe fn iter_next(&self, _index: usize, heap: &'v Heap) -> Option<values::Value<'v>> {
        let mut buf = vec![0; 65536];
        let r = match &self.readable {
            stream::Readable::Stdin(stdin) => stdin.lock().read(&mut buf),
            stream::Readable::ChildStderr(err) => err.lock().unwrap().borrow_mut().read(&mut buf),
            stream::Readable::ChildStdout(out) => out.lock().unwrap().borrow_mut().read(&mut buf),
        };
        if r.is_err() {
            return None;
        }
        let size = r.unwrap();
        if size == 0 {
            return None;
        }
        Some(super::super::types::bytes::Bytes::from(&buf[0..size]).alloc_value(heap))
    }
    unsafe fn iter_stop(&self) {}
}

starlark_simple_value!(ReadIterable);
