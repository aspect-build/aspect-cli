use allocative::Allocative;
use derive_more::Display;
use starlark::environment::GlobalsBuilder;
use starlark::starlark_module;
use starlark::starlark_simple_value;
use starlark::values;
use starlark::values::starlark_value;
use starlark::values::NoSerialize;
use starlark::values::ProvidesStaticType;
use starlark::values::Trace;

use std::fmt::Debug;
use std::sync::atomic::AtomicU64;
use std::sync::atomic::Ordering;
use std::thread::sleep;
use std::time::Duration;

use starlark::typing::Ty;
use starlark::values::Heap;

#[derive(ProvidesStaticType, Display, Trace, NoSerialize, Allocative, Debug)]
#[display("<clock_iterator>")]

struct ClockIterator {
    rate: u64,
    counter: AtomicU64,
}

starlark_simple_value!(ClockIterator);

#[starlark_value(type = "clock_iterator")]
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
        _heap: &'v Heap,
    ) -> starlark::Result<values::Value<'v>> {
        Ok(me)
    }
    unsafe fn iter_next(&self, _index: usize, heap: &'v Heap) -> Option<values::Value<'v>> {
        sleep(Duration::from_millis(self.rate));
        Some(heap.alloc(self.counter.fetch_add(1, Ordering::Relaxed)))
    }
    unsafe fn iter_stop(&self) {}
}

#[starlark_module]
pub fn register_toplevels(_: &mut GlobalsBuilder) {
    /// Forever clock iterator
    fn forever<'v>(#[starlark()] rate: u32) -> starlark::Result<ClockIterator> {
        Ok(ClockIterator {
            rate: rate as u64,
            counter: AtomicU64::new(0),
        })
    }
}
