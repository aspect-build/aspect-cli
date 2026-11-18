//! The release of the Bazel binary the runtime runs, asked once per process:
//! `bazel --version`, which the client answers without a server, else
//! `bazel info` under startup flags the invocation uses.

use std::io;
use std::sync::{Arc, OnceLock};

use bazel::server::Server;

use super::runner::SignalRunner;
use crate::engine::cancellation::Signals;

/// The memo: the first answer a caller can stand on (see [`get`]).
static VERSION: OnceLock<Option<semver::Version>> = OnceLock::new();

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
/// server the invocation uses. That answer, or its failure, is memoized too,
/// so callers that degrade to the latest do not ask again. A caller that does
/// not know them (`None`) gets `None`, left unmemoized for one that does.
pub(crate) fn get(signals: &Arc<Signals>, startup: Option<&[String]>) -> Option<semver::Version> {
    let runner = SignalRunner(signals);
    remembered(
        &VERSION,
        || bazel::version::client_version(&runner),
        startup.map(|startup| {
            let runner = &runner;
            move || {
                Server::new(runner, startup)
                    .info()
                    .inspect_err(|e| tracing::debug!("bazel info: {e}; assuming the latest"))
                    .ok()
                    .and_then(|(_pid, version)| version)
            }
        }),
    )
}

/// [`get`]'s rule over `cell`: `client`'s answer, else
/// `fallback`'s when there is one, each memoized; with no fallback a failed
/// `client` is `None` and left unmemoized.
fn remembered(
    cell: &OnceLock<Option<semver::Version>>,
    client: impl FnOnce() -> io::Result<Option<semver::Version>>,
    fallback: Option<impl FnOnce() -> Option<semver::Version>>,
) -> Option<semver::Version> {
    if let Some(version) = cell.get() {
        return version.clone();
    }
    let answer = match client() {
        Ok(version) => Some(version),
        Err(e) => {
            tracing::debug!("bazel --version: {e}");
            fallback.map(|ask| ask())
        }
    };
    answer.and_then(|version| cell.get_or_init(|| version).clone())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn v(s: &str) -> Option<semver::Version> {
        semver::Version::parse(s).ok()
    }

    fn failed() -> io::Result<Option<semver::Version>> {
        Err(io::Error::other("wrapper"))
    }

    /// A failed `--version` with no startup flags to ask the server under is
    /// left for a caller that has them, whose answer is then everyone's.
    #[test]
    fn an_answer_is_memoized_only_once_a_caller_can_stand_on_it() {
        let cell = OnceLock::new();
        let none: Option<fn() -> Option<semver::Version>> = None;
        assert_eq!(remembered(&cell, failed, none), None);
        assert_eq!(cell.get(), None, "nothing to remember yet");
        assert_eq!(remembered(&cell, failed, Some(|| v("7.6.0"))), v("7.6.0"));
        assert_eq!(remembered(&cell, || Ok(v("9.0.0")), none), v("7.6.0"));
    }

    /// `--version`'s answer, a non-release one included, needs no fallback.
    #[test]
    fn the_clients_answer_is_memoized() {
        let cell = OnceLock::new();
        let asked = std::cell::Cell::new(false);
        let fallback = Some(|| {
            asked.set(true);
            v("1.0.0")
        });
        assert_eq!(remembered(&cell, || Ok(None), fallback), None);
        assert!(!asked.get());
        assert_eq!(cell.get(), Some(&None));
    }
}
