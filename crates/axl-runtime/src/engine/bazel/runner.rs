//! The runtime's [`bazel::runner::Runner`]: bazel clients are spawned bound to
//! the cancellation root, so a cancel stops them and a wait on one is a safe
//! point.

use std::io;
use std::process::{Command, ExitStatus, Output};
use std::sync::{Arc, OnceLock};

use bazel::runner::{Child, Runner};
use bazel::server::Server;

use crate::engine::cancellation::Signals;
use crate::engine::children::{self, Spawned};

pub(crate) struct SignalRunner<'a>(pub(crate) &'a Arc<Signals>);

impl Runner for SignalRunner<'_> {
    /// Honors the `BAZEL_REAL` env var (the bazelisk convention) so wrapped
    /// invocations and tests can substitute their own binary; falls back to
    /// plain `"bazel"` and lets the OS resolve it via `PATH`.
    fn binary(&self) -> String {
        std::env::var("BAZEL_REAL").unwrap_or_else(|_| "bazel".to_string())
    }

    fn spawn(&self, cmd: &mut Command) -> io::Result<Box<dyn Child>> {
        Ok(Box::new(children::spawn_bazel(cmd, self.0)?))
    }

    fn cancelled(&self) -> bool {
        self.0.should_unwind()
    }
}

/// Process-wide cache of the Bazel release version, populated on first use.
static RELEASE_VERSION: OnceLock<Option<semver::Version>> = OnceLock::new();

/// The Bazel release version, probed once per process via
/// [`Server::info`] and memoized. `None` when Bazel reports a non-release
/// build or the probe fails — callers treat that as "version unknown" (assume
/// latest for version-gated flags).
///
/// The version is stable for the lifetime of an `aspect` invocation, so the
/// single probe is shared by every caller (flag-gating, rc-section selection,
/// …) instead of each shelling out to Bazel independently.
pub(crate) fn release_version(signals: &Arc<Signals>) -> Option<semver::Version> {
    RELEASE_VERSION
        .get_or_init(|| {
            Server::new(&SignalRunner(signals), &[])
                .info()
                .ok()
                .and_then(|(_pid, version)| version)
        })
        .clone()
}

impl Child for Spawned {
    fn wait(&self) -> io::Result<ExitStatus> {
        Spawned::wait(self)
    }

    fn wait_with_output(self: Box<Self>) -> io::Result<Output> {
        Spawned::wait_with_output(*self)
    }
}
