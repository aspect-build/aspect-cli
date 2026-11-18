//! The runtime's [`bazel::runner::Runner`]: bazel clients are spawned bound to
//! the cancellation root, so a cancel stops them and a wait on one is a safe
//! point.

use std::collections::HashMap;
use std::io;
use std::process::{Command, ExitStatus, Output};
use std::sync::{Arc, Mutex, OnceLock};

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

/// Process-wide cache of the Bazel release version, per startup flag list.
static RELEASE_VERSION: OnceLock<Mutex<HashMap<Vec<String>, Option<semver::Version>>>> =
    OnceLock::new();

/// The release version of the Bazel server `startup` reaches, asked with
/// [`Server::info`] once per process for each startup flag list and memoized;
/// `Ok(None)` for a non-release build, which callers treat as the latest.
///
/// `startup` must be the invocation's own: any other list starts, or with
/// other server options restarts, a server the invocation does not use. The
/// version is stable for the lifetime of an `aspect` invocation, so every
/// caller (flag gating, rc-section selection, `ctx.bazel.version()`, each
/// retry) shares one probe. A failed probe is not cached.
pub(crate) fn release_version(
    signals: &Arc<Signals>,
    startup: &[String],
) -> anyhow::Result<Option<semver::Version>> {
    let cache = RELEASE_VERSION.get_or_init(Default::default);
    if let Some(version) = cache.lock().unwrap().get(startup) {
        return Ok(version.clone());
    }
    let version = Server::new(&SignalRunner(signals), startup)
        .info()
        .map_err(|e| anyhow::Error::new(e).context("failed to get Bazel server info"))?
        .1;
    cache
        .lock()
        .unwrap()
        .insert(startup.to_vec(), version.clone());
    Ok(version)
}

impl Child for Spawned {
    fn wait(&self) -> io::Result<ExitStatus> {
        Spawned::wait(self)
    }

    fn wait_with_output(self: Box<Self>) -> io::Result<Output> {
        Spawned::wait_with_output(*self)
    }
}
