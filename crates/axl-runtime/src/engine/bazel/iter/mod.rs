//! The iterator handles a task binds to one build's decoded logs:
//! [`ExecLogIter`] and [`WorkspaceEventIter`], each over a
//! [`Subscription`](bazel::subscription::Subscription).
//!
//! A blocking read is cancellation-aware, as every blocking builtin must be
//! (see `engine::cancellation`), and here that earns its keep: a drain sits on
//! the critical path of every bazel-driving task, and a log ends only once the
//! client has exited and the file is no longer held or has gone quiet (see
//! `bazel::stream`), which an interrupted invocation need not make happen
//! promptly. A cancel ends the iteration, which drops the subscriber and so
//! frees a reader blocked on it; the task's next call raises its exit, since an
//! iterator cannot raise one itself.
//!
//! A handle mid-read cannot be bound to another build. AXL cannot reach that
//! state, being single-threaded: a task cannot call `ctx.bazel.build` while
//! parked in `iter_next`. `a_handle_mid_read_cannot_be_rebound` asserts it on
//! [`ExecLogIter`] anyway.

pub mod execlog;
pub mod workspace_event;

pub use execlog::ExecLogIter;
pub use workspace_event::WorkspaceEventIter;

use fibre::spmc::Receiver;

/// An iterator handle `Build::spawn` subscribes to one of a build's decoded
/// logs, and `wait()` releases.
pub trait Handle<T: Send + Clone> {
    /// Subscribe to `recv`. Fails for a handle already bound.
    fn bind(&self, recv: Receiver<T>) -> anyhow::Result<()>;
    /// Unsubscribe. Idempotent.
    fn release(&self);
}
