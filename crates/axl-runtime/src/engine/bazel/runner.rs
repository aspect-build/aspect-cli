//! The runtime's [`bazel::runner::Runner`]: bazel clients are spawned bound to
//! the cancellation root, so a cancel stops them and a wait on one is a safe
//! point. Also the process's memo of the Bazel release version
//! ([`release_version`]), so callers share one `bazel info`.

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

/// The Bazel release version per startup flag list, memoized for the process.
type VersionCache = Mutex<HashMap<Vec<String>, Option<semver::Version>>>;

static RELEASE_VERSION: OnceLock<VersionCache> = OnceLock::new();

/// The release version of the Bazel server `startup` reaches, asked with
/// [`Server::info`] once per process for each startup flag list; `None` for a
/// non-release build or a failed probe, which callers treat as the latest.
///
/// `startup` must be the invocation's own: any other list starts, or with
/// other server options restarts, a server the invocation does not use. The
/// probe runs in the process's working directory, as `Build::spawn`'s own
/// does, so a call with another `directory=` gets this workspace's Bazel. The
/// version is stable for the lifetime of an `aspect` invocation, so every
/// caller (flag gating, rc-section selection, `ctx.bazel.version()`, each
/// retry) shares one probe.
pub(crate) fn release_version(
    signals: &Arc<Signals>,
    startup: &[String],
) -> Option<semver::Version> {
    memoized(
        RELEASE_VERSION.get_or_init(Default::default),
        startup,
        || {
            Server::new(&SignalRunner(signals), startup)
                .info()
                .ok()
                .and_then(|(_pid, version)| version)
        },
    )
}

/// Record the version a `bazel info` under `startup` reported, so later
/// [`release_version`] calls for the same list reuse it.
pub(crate) fn remember_release_version(startup: &[String], version: Option<semver::Version>) {
    RELEASE_VERSION
        .get_or_init(Default::default)
        .lock()
        .unwrap()
        .insert(startup.to_vec(), version);
}

/// `cache`'s answer for `startup`, from `probe` the first time.
fn memoized(
    cache: &VersionCache,
    startup: &[String],
    probe: impl FnOnce() -> Option<semver::Version>,
) -> Option<semver::Version> {
    if let Some(version) = cache.lock().unwrap().get(startup) {
        return version.clone();
    }
    let version = probe();
    cache
        .lock()
        .unwrap()
        .insert(startup.to_vec(), version.clone());
    version
}

impl Child for Spawned {
    fn wait(&self) -> io::Result<ExitStatus> {
        Spawned::wait(self)
    }

    fn wait_with_output(self: Box<Self>) -> io::Result<Output> {
        Spawned::wait_with_output(*self)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn flags(items: &[&str]) -> Vec<String> {
        items.iter().map(|s| s.to_string()).collect()
    }

    /// Each startup flag list is asked once, a non-release answer included,
    /// and lists do not share an answer.
    #[test]
    fn a_version_is_asked_once_per_startup_flag_list() {
        let cache = VersionCache::default();
        let calls = std::cell::Cell::new(0);
        let probe = |v: Option<&str>| {
            calls.set(calls.get() + 1);
            v.map(|v| semver::Version::parse(v).unwrap())
        };
        let a = flags(&["--output_base=/a"]);
        let b = flags(&["--output_base=/b"]);
        assert_eq!(
            memoized(&cache, &a, || probe(Some("8.1.0"))),
            expected("8.1.0")
        );
        assert_eq!(
            memoized(&cache, &a, || probe(Some("9.0.0"))),
            expected("8.1.0")
        );
        assert_eq!(memoized(&cache, &b, || probe(None)), None);
        assert_eq!(memoized(&cache, &b, || probe(Some("9.0.0"))), None);
        assert_eq!(calls.get(), 2);
    }

    fn expected(v: &str) -> Option<semver::Version> {
        Some(semver::Version::parse(v).unwrap())
    }
}
