//! The runtime's [`bazel::runner::Runner`]: bazel clients are spawned bound to
//! the cancellation root, so a cancel stops them and a wait on one is a safe
//! point. Also the process's memo of the Bazel release version
//! ([`release_version`]), so callers share one `bazel --version`.

use std::io;
use std::process::{Command, ExitStatus, Output};
use std::sync::{Arc, OnceLock};

use bazel::runner::{Child, Runner};

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

/// The release of the Bazel binary the runtime runs, asked once per process
/// with [`bazel::server::client_version`] and memoized; `None` for a
/// non-release build or a failed probe, which callers treat as the latest.
///
/// `bazel --version` is answered by the client alone, so asking starts or
/// restarts no server, whatever startup flags the invocation then uses, and
/// every caller (flag gating, rc-section selection, `ctx.bazel.version()`)
/// shares the one answer.
pub(crate) fn release_version(signals: &Arc<Signals>) -> Option<semver::Version> {
    RELEASE_VERSION
        .get_or_init(|| {
            bazel::server::client_version(&SignalRunner(signals))
                .ok()
                .flatten()
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
