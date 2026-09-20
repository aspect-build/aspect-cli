use std::cell::RefCell;
use std::sync::Arc;
use std::sync::Mutex;

use allocative::Allocative;
use fibre::RecvError;
use fibre::TryRecvError;
use starlark::StarlarkResultExt;
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

/// Set by a decoder thread when it stops early. `None` means the stream ended
/// because it ran out of entries, which is the only clean way for it to end.
pub type DecodeFailure = Arc<Mutex<Option<String>>>;

#[derive(ProvidesStaticType, Display, Trace, NoSerialize, Allocative, Debug)]
#[display("<execlog_iterator>")]
pub struct ExecutionLogIterator {
    #[allocative(skip)]
    recv: RefCell<Receiver<ExecLogEntry>>,
    /// Only populated when the entries come from a file read that owns its own
    /// decoder thread. The live build path reports its errors through the build.
    #[allocative(skip)]
    failure: Option<DecodeFailure>,
}

impl ExecutionLogIterator {
    pub fn new(recv: Receiver<ExecLogEntry>) -> Self {
        Self {
            recv: RefCell::new(recv),
            failure: None,
        }
    }

    /// An iterator whose producer reports decode failures through `failure`.
    pub fn with_failure_slot(recv: Receiver<ExecLogEntry>, failure: DecodeFailure) -> Self {
        Self {
            recv: RefCell::new(recv),
            failure: Some(failure),
        }
    }
}

impl<'v> AllocValue<'v> for ExecutionLogIterator {
    fn alloc_value(self, heap: Heap<'v>) -> values::Value<'v> {
        heap.alloc_complex_no_freeze(self)
    }
}

#[starlark_module]
pub(crate) fn execlog_methods(registry: &mut MethodsBuilder) {
    /// Returns `ExecLogEntry` if event buffer is not empty.
    /// Maximum `1000` events is buffered at once.
    fn try_pop<'v>(this: values::Value<'v>) -> anyhow::Result<NoneOr<ExecLogEntry>> {
        let this = this
            .downcast_ref_err::<ExecutionLogIterator>()
            .into_anyhow_result()?;
        match this.recv.borrow_mut().try_recv() {
            Ok(it) => Ok(NoneOr::Other(it)),
            Err(TryRecvError::Empty) => Ok(NoneOr::None),
            Err(TryRecvError::Disconnected) => Ok(NoneOr::None),
        }
    }

    /// Returns `True` if stream is complete and all the events are received via `for`
    /// or calling `try_pop` repeatedly.
    fn done<'v>(this: values::Value<'v>) -> anyhow::Result<bool> {
        let this = this
            .downcast_ref_err::<ExecutionLogIterator>()
            .into_anyhow_result()?;
        Ok(this.recv.borrow().is_closed())
    }

    /// Why the stream stopped early, or `None` if it ran to the end of the log.
    ///
    /// Iteration ends silently when the decoder gives up, so a caller that must not
    /// act on a partial log checks this once the loop is over. Always `None` for a
    /// stream attached to a running build, where a decode failure fails the build.
    ///
    /// ```python
    /// entries = bazel.execution_log.read(path = "a.binpb.zst")
    /// for entry in entries:
    ///     ...
    /// if entries.error() != None:
    ///     ctx.std.process.exit(1, "a.binpb.zst: " + entries.error())
    /// ```
    fn error<'v>(this: values::Value<'v>) -> anyhow::Result<NoneOr<String>> {
        let this = this
            .downcast_ref_err::<ExecutionLogIterator>()
            .into_anyhow_result()?;
        let Some(failure) = this.failure.as_ref() else {
            return Ok(NoneOr::None);
        };
        let failure = failure
            .lock()
            .map_err(|e| anyhow::anyhow!("execution log decoder state was poisoned: {e}"))?;
        Ok(match failure.as_ref() {
            Some(msg) => NoneOr::Other(msg.clone()),
            None => NoneOr::None,
        })
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
        _heap: Heap<'v>,
    ) -> starlark::Result<values::Value<'v>> {
        Ok(me)
    }
    unsafe fn iter_next(&self, _index: usize, heap: Heap<'v>) -> Option<values::Value<'v>> {
        match self.recv.borrow_mut().recv() {
            Ok(ev) => Some(ev.alloc_value(heap)),
            Err(RecvError::Disconnected) => None,
        }
    }
    unsafe fn iter_stop(&self) {}
}
