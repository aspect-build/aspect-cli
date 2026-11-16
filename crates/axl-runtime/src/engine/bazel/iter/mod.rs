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

impl Handle<axl_proto::tools::protos::ExecLogEntry> for ExecLogIter {
    fn bind(&self, recv: Receiver<axl_proto::tools::protos::ExecLogEntry>) -> anyhow::Result<()> {
        ExecLogIter::bind(self, recv)
    }
    fn release(&self) {
        ExecLogIter::release(self)
    }
}

impl Handle<axl_proto::workspace_log::WorkspaceEvent> for WorkspaceEventIter {
    fn bind(&self, recv: Receiver<axl_proto::workspace_log::WorkspaceEvent>) -> anyhow::Result<()> {
        WorkspaceEventIter::bind(self, recv)
    }
    fn release(&self) {
        WorkspaceEventIter::release(self)
    }
}
