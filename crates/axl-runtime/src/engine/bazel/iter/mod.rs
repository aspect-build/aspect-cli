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
