use allocative::Allocative;
use axl_proto::build_event_stream::BuildEvent;

use derive_more::Display;
use fibre::spmc::Receiver;
use starlark::values::none::NoneOr;
use std::cell::RefCell;
use std::fmt::Debug;
use std::fs::File;
use std::io::BufReader;
use zstd::Decoder;

use starlark::environment::Methods;
use starlark::environment::MethodsBuilder;
use starlark::environment::MethodsStatic;
use starlark::starlark_module;
use starlark::typing::Ty;
use starlark::values;
use starlark::values::starlark_value;
use starlark::values::AllocValue;
use starlark::values::Heap;
use starlark::values::NoSerialize;
use starlark::values::ProvidesStaticType;
use starlark::values::Trace;
use starlark::values::ValueLike;

use crate::engine::bazel::execlog_stream::{ExecLogIterator, RetryStream};

#[derive(Debug, ProvidesStaticType, Display, Trace, NoSerialize, Allocative)]
#[display("<build_event_iterator>")]
pub struct BuildEventIterator {
    #[allocative(skip)]
    recv: RefCell<Receiver<BuildEvent>>,
}

impl BuildEventIterator {
    pub fn new(recv: Receiver<BuildEvent>) -> Self {
        Self {
            recv: RefCell::new(recv),
        }
    }
}

impl<'v> AllocValue<'v> for BuildEventIterator {
    fn alloc_value(self, heap: &'v Heap) -> values::Value<'v> {
        heap.alloc_complex_no_freeze(self)
    }
}

#[starlark_module]
pub(crate) fn build_event_methods(registry: &mut MethodsBuilder) {
    fn poll<'v>(this: values::Value<'v>) -> anyhow::Result<NoneOr<BuildEvent>> {
        let this = this.downcast_ref_err::<BuildEventIterator>()?;
        match this.recv.borrow_mut().try_recv() {
            Ok(it) => Ok(NoneOr::Other(it)),
            Err(_) => Ok(NoneOr::None),
        }
    }
}

#[starlark_value(type = "build_event_iterator")]
impl<'v> values::StarlarkValue<'v> for BuildEventIterator {
    fn eval_type(&self) -> Option<Ty> {
        Some(Ty::iter(
            axl_proto::build_event_stream::BuildEvent::get_type_starlark_repr(),
        ))
    }

    fn get_type_starlark_repr() -> Ty {
        Ty::iter(axl_proto::build_event_stream::BuildEvent::get_type_starlark_repr())
    }

    fn get_methods() -> Option<&'static Methods> {
        static RES: MethodsStatic = MethodsStatic::new();
        RES.methods(build_event_methods)
    }

    unsafe fn iterate(
        &self,
        me: values::Value<'v>,
        _heap: &'v Heap,
    ) -> starlark::Result<values::Value<'v>> {
        Ok(me)
    }
    unsafe fn iter_next(&self, _index: usize, heap: &'v Heap) -> Option<values::Value<'v>> {
        self.recv
            .borrow_mut()
            .recv()
            .ok()
            .map(|event| heap.alloc(event))
    }
    unsafe fn iter_stop(&self) {}
}

#[derive(ProvidesStaticType, Display, Trace, NoSerialize, Allocative)]
#[display("<build_event_iterator>")]
pub struct ExecutionLogIterator {
    #[allocative(skip)]
    execlog_iterator:
        RefCell<ExecLogIterator<RetryStream<Decoder<'static, BufReader<RetryStream<File>>>>>>,
}

impl Debug for ExecutionLogIterator {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ExecutionLogIterator")
            .field("execlog_iterator", &String::from("hidden"))
            .finish()
    }
}

impl ExecutionLogIterator {
    pub fn new(backing_file: File) -> anyhow::Result<Self> {
        let retry_stream = RetryStream {
            inner: backing_file,
        };
        let decoder = Decoder::new(retry_stream)?;
        let out = RetryStream { inner: decoder };
        let iter = ExecLogIterator::new(out);
        Ok(Self {
            execlog_iterator: RefCell::new(iter),
        })
    }
}

impl<'v> AllocValue<'v> for ExecutionLogIterator {
    fn alloc_value(self, heap: &'v Heap) -> values::Value<'v> {
        heap.alloc_complex_no_freeze(self)
    }
}

#[starlark_value(type = "build_event_iterator")]
impl<'v> values::StarlarkValue<'v> for ExecutionLogIterator {
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
        self.execlog_iterator
            .borrow_mut()
            .next()
            .map(|ev| heap.alloc(ev))
    }
    unsafe fn iter_stop(&self) {}
}
