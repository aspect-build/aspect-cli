//! [`WorkspaceEventIter`] backs `bazel.workspace_events.iterator()`: created
//! before the spawn and passed in `workspace_events = [...]`.
//!
//! A bound handle is a subscriber the producer blocks for, so `build.wait()`
//! releases every one still held before joining the reader, which is what
//! bounds a consumer that stopped reading. That lifecycle, and the
//! cancellation-aware blocking read it wraps, live in [`Subscription`].

use std::sync::Arc;

use allocative::Allocative;
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
use starlark::values::UnpackValue;
use starlark::values::ValueLike;
use starlark::values::none::NoneOr;
use starlark::values::none::NoneType;
use starlark::values::starlark_value;

use axl_proto::workspace_log::WorkspaceEvent;
use bazel::subscription::Subscription;
use derive_more::Display;
use fibre::spmc::Receiver;

use super::Handle;
use crate::engine::cancellation::Signals;

/// A `bazel.workspace_events.iterator()` handle: a subscriber to one build's
/// workspace event log, created before the spawn and passed in
/// `workspace_events = [...]`. Every handle receives every event.
#[derive(Clone, Debug, ProvidesStaticType, Display, Trace, NoSerialize, Allocative)]
#[display("<bazel.workspace_events.WorkspaceEventIter>")]
pub struct WorkspaceEventIter {
    /// Starts [`Subscription::pending`]: this handle exists before the build it
    /// will be bound to.
    #[allocative(skip)]
    #[trace(unsafe_ignore)]
    sub: Subscription<WorkspaceEvent>,
    /// The run's cancellation state, so blocking iteration stays a safe point.
    #[allocative(skip)]
    signals: Arc<Signals>,
}

impl WorkspaceEventIter {
    pub fn new(signals: Arc<Signals>) -> Self {
        Self {
            sub: Subscription::pending(),
            signals,
        }
    }

    /// Whether this handle still holds a subscriber; `false` once released or
    /// finished. For tests.
    #[cfg(test)]
    pub fn is_live(&self) -> bool {
        self.sub.is_live()
    }
}

impl Handle<WorkspaceEvent> for WorkspaceEventIter {
    /// Subscribe this handle to a build's workspace event log. One handle binds
    /// once, so a handle reused across two builds is an error rather than a
    /// stream silently attached to the wrong one.
    fn bind(&self, recv: Receiver<WorkspaceEvent>) -> anyhow::Result<()> {
        if self.sub.bind(recv) {
            return Ok(());
        }
        anyhow::bail!(
            "this `bazel.workspace_events.iterator()` handle was already bound or has finished; \
             create a fresh one per build",
        )
    }

    /// Unsubscribe, dropping any events still buffered. `build.wait()` calls
    /// this before joining the stream, so an undrained handle cannot park the
    /// reader.
    fn release(&self) {
        self.sub.release();
    }
}

impl<'v> AllocValue<'v> for WorkspaceEventIter {
    fn alloc_value(self, heap: Heap<'v>) -> values::Value<'v> {
        heap.alloc_complex_no_freeze(self)
    }
}

impl<'v> UnpackValue<'v> for WorkspaceEventIter {
    type Error = anyhow::Error;
    fn unpack_value_impl(value: values::Value<'v>) -> Result<Option<Self>, Self::Error> {
        let v = value
            .downcast_ref_err::<WorkspaceEventIter>()
            .into_anyhow_result()?;
        Ok(Some(v.clone()))
    }
}

#[starlark_value(type = "bazel.workspace_events.WorkspaceEventIter")]
impl<'v> values::StarlarkValue<'v> for WorkspaceEventIter {
    fn eval_type(&self) -> Option<Ty> {
        Some(Ty::iter(WorkspaceEvent::get_type_starlark_repr()))
    }

    fn get_methods() -> Option<&'static Methods> {
        static RES: MethodsStatic =
            MethodsStatic::new("workspace_event_iter_methods", workspace_event_iter_methods);
        Some(RES.methods())
    }

    /// `done` means "this handle no longer holds a subscriber": released by
    /// `drain()`, a cancel, the end of the stream, or `build.wait()`.
    fn get_attr(&self, attribute: &str, heap: Heap<'v>) -> Option<values::Value<'v>> {
        match attribute {
            "done" => Some(heap.alloc(self.sub.is_done())),
            _ => None,
        }
    }

    fn has_attr(&self, attribute: &str, _heap: Heap<'v>) -> bool {
        matches!(attribute, "done")
    }

    unsafe fn iterate(
        &self,
        me: values::Value<'v>,
        _heap: Heap<'v>,
    ) -> starlark::Result<values::Value<'v>> {
        Ok(me)
    }

    /// Blocks until the next event or the end of the stream. See
    /// the [`iter`](super) module docs for why it is cancellation-aware.
    unsafe fn iter_next(&self, _index: usize, heap: Heap<'v>) -> Option<values::Value<'v>> {
        self.sub
            .recv_blocking(|| self.signals.should_unwind(), |_| true)
            .map(|event| event.alloc_value(heap))
    }

    /// Leaving a `for` loop is not unsubscribing: `try_pop` and a second loop
    /// resume where the first stopped.
    unsafe fn iter_stop(&self) {}
}

#[starlark_module]
pub(crate) fn workspace_event_iter_methods(registry: &mut MethodsBuilder) {
    /// Non-blocking pop of the next event, or `None` when nothing is buffered
    /// yet.
    fn try_pop<'v>(this: values::Value<'v>) -> anyhow::Result<NoneOr<WorkspaceEvent>> {
        let iter = this
            .downcast_ref_err::<WorkspaceEventIter>()
            .into_anyhow_result()?;
        Ok(NoneOr::from_option(iter.sub.try_pop(|_| true)))
    }

    /// Stop consuming: unsubscribe and drop whatever is buffered. Idempotent.
    fn drain<'v>(this: values::Value<'v>) -> anyhow::Result<NoneType> {
        let iter = this
            .downcast_ref_err::<WorkspaceEventIter>()
            .into_anyhow_result()?;
        iter.release();
        Ok(NoneType)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use fibre::spmc::bounded;

    #[test]
    fn a_handle_binds_once() {
        let (_s1, r1) = bounded::<WorkspaceEvent>(4);
        let (_s2, r2) = bounded::<WorkspaceEvent>(4);
        let iter = WorkspaceEventIter::new(Signals::new());
        iter.bind(r1).unwrap();
        let err = iter
            .bind(r2)
            .expect_err("a second bind must be refused")
            .to_string();
        assert!(err.contains("already bound"), "unexpected error: {err}");
    }

    #[test]
    fn a_released_handle_holds_no_subscriber_and_pops_nothing() {
        let (sender, recv) = bounded::<WorkspaceEvent>(4);
        let iter = WorkspaceEventIter::new(Signals::new());
        iter.bind(recv).unwrap();
        sender.send(WorkspaceEvent::default()).unwrap();
        assert!(iter.is_live());
        iter.release();
        assert!(!iter.is_live());
        assert!(iter.sub.try_pop(|_| true).is_none());
    }
}
