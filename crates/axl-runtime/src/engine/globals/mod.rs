use allocative::Allocative;
use derive_more::Display;
use starlark::environment::GlobalsBuilder;
use starlark::starlark_module;
use starlark::starlark_simple_value;
use starlark::values;
use starlark::values::NoSerialize;
use starlark::values::ProvidesStaticType;
use starlark::values::Trace;
use starlark::values::starlark_value;

use std::fmt::Debug;
use std::sync::atomic::AtomicU64;
use std::sync::atomic::Ordering;
use std::thread::sleep;
use std::time::Duration;

use starlark::typing::Ty;
use starlark::values::Heap;

#[derive(ProvidesStaticType, Display, Trace, NoSerialize, Allocative, Debug)]
#[display("<ClockIterator>")]

struct ClockIterator {
    rate: u64,
    counter: AtomicU64,
}

starlark_simple_value!(ClockIterator);

#[starlark_value(type = "ClockIterator")]
impl<'v> values::StarlarkValue<'v> for ClockIterator {
    fn eval_type(&self) -> Option<Ty> {
        Some(Ty::iter(
            axl_proto::tools::protos::ExecLogEntry::get_type_starlark_repr(),
        ))
    }

    fn get_type_starlark_repr() -> Ty {
        Ty::iter(axl_proto::tools::protos::ExecLogEntry::get_type_starlark_repr())
    }

    unsafe fn iterate(
        &self,
        me: values::Value<'v>,
        _heap: Heap<'v>,
    ) -> starlark::Result<values::Value<'v>> {
        Ok(me)
    }
    unsafe fn iter_next(&self, _index: usize, heap: Heap<'v>) -> Option<values::Value<'v>> {
        sleep(Duration::from_millis(self.rate));
        Some(heap.alloc(self.counter.fetch_add(1, Ordering::Relaxed)))
    }
    unsafe fn iter_stop(&self) {}
}

#[starlark_module]
pub fn register_globals(globals: &mut GlobalsBuilder) {
    /// Forever clock iterator
    fn forever<'v>(#[starlark()] rate: u32) -> anyhow::Result<ClockIterator> {
        Ok(ClockIterator {
            rate: rate as u64,
            counter: AtomicU64::new(0),
        })
    }

    /// Creates a `Bytes` value from a hex-encoded string.
    fn bytes<'v>(hex: &str, heap: Heap<'v>) -> anyhow::Result<starlark::values::Value<'v>> {
        if hex.len() % 2 != 0 {
            return Err(anyhow::anyhow!("hex string must have even length"));
        }
        let data = (0..hex.len())
            .step_by(2)
            .map(|i| {
                u8::from_str_radix(&hex[i..i + 2], 16)
                    .map_err(|e| anyhow::anyhow!("bad hex at position {}: {}", i, e))
            })
            .collect::<anyhow::Result<Vec<u8>>>()?;
        Ok(heap.alloc(starlark::values::bytes::StarlarkBytes::new(&data)))
    }
}
