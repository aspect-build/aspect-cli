use allocative::Allocative;
use arc_slice::ArcBytes;
use derive_more::Display;
use starlark::environment::Methods;
use starlark::environment::MethodsBuilder;
use starlark::environment::MethodsStatic;
use starlark::starlark_module;
use starlark::values::AllocValue;
use starlark::values::Value;

use starlark::values;
use starlark::values::Heap;
use starlark::values::NoSerialize;
use starlark::values::ProvidesStaticType;
use starlark::values::StarlarkValue;
use starlark::values::Trace;
use starlark::values::ValueLike;
use starlark::values::starlark_value;

#[derive(Debug, ProvidesStaticType, Display, Trace, NoSerialize, Allocative)]
#[display("<Bytes>")]
pub struct Bytes {
    #[allocative(skip)]
    pub(crate) buf: ArcBytes,
}

impl From<&[u8]> for Bytes {
    fn from(value: &[u8]) -> Self {
        Self {
            buf: ArcBytes::from_slice(value),
        }
    }
}

impl<'v> AllocValue<'v> for Bytes {
    fn alloc_value(self, heap: &'v Heap) -> Value<'v> {
        heap.alloc_complex_no_freeze(self)
    }
}

#[starlark_value(type = "Bytes")]
impl<'v> StarlarkValue<'v> for Bytes {
    // Spec: The built-in len function returns the number of elements (bytes) in a bytes.
    fn length(&self) -> starlark::Result<i32> {
        Ok(self.buf.len() as i32)
    }

    // Spec: Two bytes values may be concatenated with the + operator.
    // fn add(&self, rhs: Value<'v>, heap: &'v Heap) -> Option<starlark::Result<Value<'v>>> {
    //     let rhs = Bytes::from_value(rhs)?;
    //     let slice = [self.buf.as_slice(), rhs.buf.as_slice()].concat();
    //     Some(Ok(heap.alloc_simple(Bytes {
    //         buf: ArcSlice::from_slice(slice.as_slice()),
    //     })))
    // }

    // Spec: The slice expression b[i:j] returns the subsequence of b from index i up to but not including index j.
    // The index expression b[i] returns the int value of the ith element.
    fn slice(
        &self,
        _start: Option<Value<'v>>,
        _stop: Option<Value<'v>>,
        _stride: Option<Value<'v>>,
        _heap: &'v Heap,
    ) -> starlark::Result<Value<'v>> {
        values::ValueError::unsupported(self, "[::]")
    }

    // Spec: bool(bytes) would be equivalent to len(bytes) > 0
    fn to_bool(&self) -> bool {
        self.buf.len() > 0
    }

    // See: https://github.com/facebook/starlark-rust/issues/4#issuecomment-3420078819
    fn collect_repr(&self, collector: &mut String) {
        collector.push_str(&String::from_utf8_lossy(&self.buf));
    }

    fn get_methods() -> Option<&'static Methods> {
        static RES: MethodsStatic = MethodsStatic::new();
        RES.methods(bytes_methods)
    }
}

#[starlark_module]
fn bytes_methods(registry: &mut MethodsBuilder) {
    /// Decodes the bytes as a UTF-8 string.
    ///
    /// Invalid UTF-8 sequences are replaced with the Unicode replacement character (U+FFFD).
    fn decode<'v>(this: values::Value) -> anyhow::Result<String> {
        let bytes = this.downcast_ref_err::<Bytes>()?;
        Ok(String::from_utf8_lossy(&bytes.buf).into_owned())
    }
}
