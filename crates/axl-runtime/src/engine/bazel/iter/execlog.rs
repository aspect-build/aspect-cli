use std::cell::RefCell;

use allocative::Allocative;
use fibre::RecvError;
use fibre::TryRecvError;
use starlark::environment::Methods;
use starlark::environment::MethodsBuilder;
use starlark::environment::MethodsStatic;
use starlark::starlark_module;
use starlark::typing::Ty;
use starlark::values;
use starlark::values::AllocValue;
use starlark::values::Heap;
use starlark::values::NoSerialize;
use starlark::values::ProvidesStaticType;
use starlark::values::Trace;
use starlark::values::ValueLike;
use starlark::values::none::NoneOr;
use starlark::values::starlark_value;

use axl_proto::tools::protos::ExecLogEntry;
use derive_more::Display;
use fibre::spmc::Receiver;

#[derive(ProvidesStaticType, Display, Trace, NoSerialize, Allocative, Debug)]
#[display("<execlog_iterator>")]
pub struct ExecutionLogIterator {
    #[allocative(skip)]
    recv: RefCell<Receiver<ExecLogEntry>>,
}

impl ExecutionLogIterator {
    pub fn new(recv: Receiver<ExecLogEntry>) -> Self {
        Self {
            recv: RefCell::new(recv),
        }
    }
}

impl<'v> AllocValue<'v> for ExecutionLogIterator {
    fn alloc_value(self, heap: &'v Heap) -> values::Value<'v> {
        heap.alloc_complex_no_freeze(self)
    }
}

#[starlark_module]
pub(crate) fn execlog_methods(registry: &mut MethodsBuilder) {
    /// Returns `ExecLogEntry` if event buffer is not empty.
    /// Maximum `1000` events is buffered at once.
    fn try_pop<'v>(this: values::Value<'v>) -> anyhow::Result<NoneOr<ExecLogEntry>> {
        let this = this.downcast_ref_err::<ExecutionLogIterator>()?;
        match this.recv.borrow_mut().try_recv() {
            Ok(it) => Ok(NoneOr::Other(it)),
            Err(TryRecvError::Empty) => Ok(NoneOr::None),
            Err(TryRecvError::Disconnected) => Ok(NoneOr::None),
        }
    }

    /// Returns `True` if stream is complete and all the events are received via `for`
    /// or calling `try_pop` repeatedly.
    fn done<'v>(this: values::Value<'v>) -> anyhow::Result<bool> {
        let this = this.downcast_ref_err::<ExecutionLogIterator>()?;
        Ok(this.recv.borrow().is_closed())
    }
}

#[starlark_value(type = "ExecutionLogIterator")]
impl<'v> values::StarlarkValue<'v> for ExecutionLogIterator {
    fn eval_type(&self) -> Option<Ty> {
        Some(Ty::iter(ExecLogEntry::get_type_starlark_repr()))
    }

    fn get_type_starlark_repr() -> Ty {
        Ty::iter(ExecLogEntry::get_type_starlark_repr())
    }

    fn get_methods() -> Option<&'static Methods> {
        static RES: MethodsStatic = MethodsStatic::new();
        RES.methods(execlog_methods)
    }

    unsafe fn iterate(
        &self,
        me: values::Value<'v>,
        _heap: &'v Heap,
    ) -> starlark::Result<values::Value<'v>> {
        Ok(me)
    }
    unsafe fn iter_next(&self, _index: usize, heap: &'v Heap) -> Option<values::Value<'v>> {
        match self.recv.borrow_mut().recv() {
            Ok(ev) => Some(ev.alloc_value(heap)),
            Err(RecvError::Disconnected) => None,
        }
    }
    unsafe fn iter_stop(&self) {}
}
