//! Starlark surface for the captured-output respond/replace flow.
//!
//! `build.output_events()` returns an [`OutputEventIterator`] over
//! [`OutputMatch`] events, one per captured line that matched a
//! `respond_patterns` regex. The line is **held** — not yet forwarded to the
//! terminal — until the event is answered with `keep()`, `replace(text)`, or
//! `drop()`, or the processor's fail-open timeout forwards the original.
//! Drain with `try_pop()` from a tick loop (or `for` to block):
//!
//! ```python
//! events = build.output_events()
//! for _tick in sleep_iter(50):
//!     ev = events.try_pop()
//!     if ev:
//!         ev.replace("(elided)") if "secret" in ev.line else ev.keep()
//!     if events.done:
//!         break
//! ```

use std::cell::{Cell, RefCell};
use std::sync::mpsc;

use allocative::Allocative;
use derive_more::Display;
use starlark::StarlarkResultExt;
use starlark::environment::Methods;
use starlark::environment::MethodsBuilder;
use starlark::environment::MethodsStatic;
use starlark::starlark_module;
use starlark::values;
use starlark::values::AllocValue;
use starlark::values::Heap;
use starlark::values::NoSerialize;
use starlark::values::ProvidesStaticType;
use starlark::values::Trace;
use starlark::values::ValueLike;
use starlark::values::none::{NoneOr, NoneType};
use starlark::values::starlark_value;

use crate::engine::bazel::stream::processors::{PendingMatch, Verdict};

/// A captured line held for a verdict. Answer exactly once with `keep()`,
/// `replace(text)`, or `drop()`; an unanswered event fails open (the original
/// line forwards after the processor's timeout).
#[derive(Debug, ProvidesStaticType, Display, Trace, NoSerialize, Allocative)]
#[display("<bazel.output.OutputMatch id={id}>")]
pub struct OutputMatch {
    id: String,
    line: String,
    #[allocative(skip)]
    reply: RefCell<Option<mpsc::SyncSender<Verdict>>>,
}

impl OutputMatch {
    fn respond(&self, verdict: Verdict) -> anyhow::Result<()> {
        let sender = self.reply.borrow_mut().take().ok_or_else(|| {
            anyhow::anyhow!("this OutputMatch was already responded to; respond exactly once")
        })?;
        // The reader may have timed out and moved on — a closed channel is
        // not an error, the verdict is simply too late to apply.
        let _ = sender.send(verdict);
        Ok(())
    }
}

impl<'v> AllocValue<'v> for OutputMatch {
    fn alloc_value(self, heap: Heap<'v>) -> values::Value<'v> {
        heap.alloc_complex_no_freeze(self)
    }
}

#[starlark_value(type = "bazel.output.OutputMatch")]
impl<'v> values::StarlarkValue<'v> for OutputMatch {
    fn get_methods() -> Option<&'static Methods> {
        static RES: MethodsStatic = MethodsStatic::new();
        RES.methods(output_match_methods)
    }

    fn get_attr(&self, attribute: &str, heap: Heap<'v>) -> Option<values::Value<'v>> {
        match attribute {
            "id" => Some(heap.alloc_str(&self.id).to_value()),
            "line" => Some(heap.alloc_str(&self.line).to_value()),
            _ => None,
        }
    }

    fn has_attr(&self, attribute: &str, _heap: Heap<'v>) -> bool {
        matches!(attribute, "id" | "line")
    }
}

#[starlark_module]
fn output_match_methods(registry: &mut MethodsBuilder) {
    /// Release the held line unchanged.
    fn keep<'v>(this: values::Value<'v>) -> anyhow::Result<NoneType> {
        let ev = this.downcast_ref_err::<OutputMatch>().into_anyhow_result()?;
        ev.respond(Verdict::Keep)?;
        Ok(NoneType)
    }

    /// Forward `text` in place of the held line (its original `\n`/`\r`
    /// boundary is preserved; embedded newlines in `text` are allowed).
    fn replace<'v>(
        this: values::Value<'v>,
        #[starlark(require = pos)] text: &str,
    ) -> anyhow::Result<NoneType> {
        let ev = this.downcast_ref_err::<OutputMatch>().into_anyhow_result()?;
        ev.respond(Verdict::Replace(text.as_bytes().to_vec()))?;
        Ok(NoneType)
    }

    /// Suppress the held line entirely (boundary included).
    fn drop<'v>(this: values::Value<'v>) -> anyhow::Result<NoneType> {
        let ev = this.downcast_ref_err::<OutputMatch>().into_anyhow_result()?;
        ev.respond(Verdict::Drop)?;
        Ok(NoneType)
    }
}

/// Iterator over [`OutputMatch`] events. Single-use handle returned by
/// `build.output_events()`. `try_pop()` is the non-blocking drain for tick
/// loops; `for` iteration blocks until the stream closes.
#[derive(Debug, ProvidesStaticType, Display, Trace, NoSerialize, Allocative)]
#[display("<bazel.output.OutputEventIterator>")]
pub struct OutputEventIterator {
    #[allocative(skip)]
    recv: RefCell<mpsc::Receiver<PendingMatch>>,
    #[allocative(skip)]
    done: Cell<bool>,
}

impl OutputEventIterator {
    pub fn new(recv: mpsc::Receiver<PendingMatch>) -> Self {
        Self {
            recv: RefCell::new(recv),
            done: Cell::new(false),
        }
    }
}

fn to_match(p: PendingMatch) -> OutputMatch {
    OutputMatch {
        id: p.id,
        line: p.line,
        reply: RefCell::new(Some(p.reply)),
    }
}

impl<'v> AllocValue<'v> for OutputEventIterator {
    fn alloc_value(self, heap: Heap<'v>) -> values::Value<'v> {
        heap.alloc_complex_no_freeze(self)
    }
}

#[starlark_module]
fn output_event_iterator_methods(registry: &mut MethodsBuilder) {
    /// Non-blocking pop: the next held-line event, or `None` when nothing is
    /// pending (or the stream has closed — check `done`).
    fn try_pop<'v>(this: values::Value<'v>) -> anyhow::Result<NoneOr<OutputMatch>> {
        let iter = this
            .downcast_ref_err::<OutputEventIterator>()
            .into_anyhow_result()?;
        match iter.recv.borrow().try_recv() {
            Ok(p) => Ok(NoneOr::Other(to_match(p))),
            Err(mpsc::TryRecvError::Empty) => Ok(NoneOr::None),
            Err(mpsc::TryRecvError::Disconnected) => {
                iter.done.set(true);
                Ok(NoneOr::None)
            }
        }
    }
}

#[starlark_value(type = "bazel.output.OutputEventIterator")]
impl<'v> values::StarlarkValue<'v> for OutputEventIterator {
    fn get_methods() -> Option<&'static Methods> {
        static RES: MethodsStatic = MethodsStatic::new();
        RES.methods(output_event_iterator_methods)
    }

    fn get_attr(&self, attribute: &str, heap: Heap<'v>) -> Option<values::Value<'v>> {
        match attribute {
            // True once the capture stream has ended and no more events can
            // arrive (set after a `try_pop` observes the disconnect).
            "done" => Some(heap.alloc(self.done.get())),
            _ => None,
        }
    }

    fn has_attr(&self, attribute: &str, _heap: Heap<'v>) -> bool {
        matches!(attribute, "done")
    }

    unsafe fn iterate(&self, me: values::Value<'v>, _heap: Heap<'v>) -> starlark::Result<values::Value<'v>> {
        Ok(me)
    }

    unsafe fn iter_next(&self, _index: usize, heap: Heap<'v>) -> Option<values::Value<'v>> {
        match self.recv.borrow().recv() {
            Ok(p) => Some(to_match(p).alloc_value(heap)),
            Err(_) => {
                self.done.set(true);
                None
            }
        }
    }

    unsafe fn iter_stop(&self) {}
}
