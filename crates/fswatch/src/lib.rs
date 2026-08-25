//! File-watching abstraction for watch mode.
//!
//! Consumers depend only on the backend-agnostic surface here ([`Subscription`],
//! [`ChangeBatch`], [`watch`]); the concrete watcher behind it is chosen by
//! [`Backend`]. Today the only backend is the in-process `notify` crate
//! (FSEvents/inotify/etc). A future backend (e.g. a watchman client) plugs in as
//! another [`Backend`] variant without touching consumers.

mod debounce;
mod notify_backend;

use std::path::PathBuf;
use std::time::Duration;

#[derive(Debug, thiserror::Error)]
pub enum WatchError {
    #[error("watch backend error: {0}")]
    Backend(String),
}

/// What happened to a path, after coalescing.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ChangeKind {
    Created,
    Modified,
    Removed,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Change {
    pub path: PathBuf,
    pub kind: ChangeKind,
}

/// A debounced, coalesced set of changes.
#[derive(Debug, Clone, Default)]
pub struct ChangeBatch {
    pub changes: Vec<Change>,
    /// The backend lost track of state (event queue overflow, daemon restart).
    /// Consumers must assume anything may have changed.
    pub rescan: bool,
}

#[derive(Debug, Clone)]
pub struct WatchConfig {
    /// The directory watched recursively.
    pub root: PathBuf,
    /// Absolute path prefixes whose events are dropped (e.g. `.git`, and the
    /// bazel convenience symlinks the caller lists by name).
    pub ignore_prefixes: Vec<PathBuf>,
    /// Quiet period that must elapse after the last event before a batch is
    /// released.
    pub debounce: Duration,
    /// Cap on how long a batch may be held back when events never go quiet.
    pub max_delay: Duration,
}

impl WatchConfig {
    pub fn new(root: PathBuf) -> Self {
        Self {
            root,
            ignore_prefixes: Vec::new(),
            debounce: Duration::from_millis(100),
            max_delay: Duration::from_secs(2),
        }
    }
}

/// A live watch. Dropping it stops the underlying watcher.
pub trait Subscription: Send {
    /// Non-blocking poll. Returns a batch once pending changes have settled
    /// for [`WatchConfig::debounce`] (or [`WatchConfig::max_delay`] elapsed),
    /// otherwise `None`.
    fn try_recv(&mut self) -> Result<Option<ChangeBatch>, WatchError>;

    /// Discard everything accumulated so far (e.g. writes caused by our own
    /// build).
    fn drain(&mut self);
}

/// Watch backend selector.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Backend {
    /// In-process watcher via the `notify` crate.
    #[default]
    Notify,
}

/// Start watching. The returned [`Subscription`] owns the backend resources.
pub fn watch(backend: Backend, config: WatchConfig) -> Result<Box<dyn Subscription>, WatchError> {
    match backend {
        Backend::Notify => notify_backend::subscribe(config),
    }
}

/// A batch whose changes were entirely filtered away (and which carries no
/// rescan) is no batch at all — returning it would trigger consumers'
/// rebuild cycles for nothing.
pub(crate) fn non_empty(batch: ChangeBatch) -> Option<ChangeBatch> {
    if batch.changes.is_empty() && !batch.rescan {
        return None;
    }
    Some(batch)
}

#[cfg(test)]
mod tests;
