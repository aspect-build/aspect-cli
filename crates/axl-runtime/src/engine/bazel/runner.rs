//! The runtime's [`bazel::runner::Runner`]: bazel clients are spawned bound to
//! the cancellation root, so a cancel stops them and a wait on one is a safe
//! point. Also the process's memo of the Bazel release version
//! ([`release_version`]), so callers share one `bazel --version`.

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

/// The release of the Bazel binary the runtime runs, asked once per process
/// with [`bazel::version::client_version`] and memoized; `None` for a
/// non-release build or a failed probe, which callers treat as the latest.
///
/// `bazel --version` itself is answered by the client alone, so it starts or
/// restarts no server, and every caller (flag gating, rc-section selection,
/// `ctx.bazel.version()`) shares its answer.
///
/// Bazel takes `--version` only alone, so a `tools/bazel` wrapper that adds a
/// startup flag of its own fails it. Then the server is asked instead, with
/// `bazel info` under `startup`: the startup flags the invocation runs with,
/// which a caller passes only when it knows them, so the question reaches the
/// server the invocation uses. A caller that does not (`None`) gets `None`,
/// left unmemoized for one that does.
pub(crate) fn release_version(
    signals: &Arc<Signals>,
    startup: Option<&[String]>,
) -> Option<semver::Version> {
    if let Some(version) = RELEASE_VERSION.get() {
        return version.clone();
    }
    let runner = SignalRunner(signals);
    let answer = match bazel::version::client_version(&runner) {
        Ok(version) => Some(version),
        Err(e) => match startup {
            Some(startup) => {
                tracing::debug!("bazel --version: {e}; asking bazel info");
                Some(
                    Server::new(&runner, startup)
                        .info()
                        .inspect_err(|e| tracing::debug!("bazel info: {e}; assuming the latest"))
                        .ok()
                        .and_then(|(_pid, version)| version),
                )
            }
            None => {
                tracing::debug!("bazel --version: {e}; assuming the latest for now");
                None
            }
        },
    };
    answer.and_then(|version| RELEASE_VERSION.get_or_init(|| version).clone())
}

impl Child for Spawned {
    fn wait(&self) -> io::Result<ExitStatus> {
        Spawned::wait(self)
    }

    fn wait_with_output(self: Box<Self>) -> io::Result<Output> {
        Spawned::wait_with_output(*self)
    }
}
