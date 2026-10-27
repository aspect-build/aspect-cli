use std::sync::{Arc, OnceLock};

use bazel::server::Server;
use starlark::collections::SmallMap;

use super::runner::SignalRunner;
use crate::engine::cancellation::Signals;

/// `bazel info` stdout as a Starlark-ready map; see
/// [`bazel::server::info::parse_output`].
pub fn parse_output<S: AsRef<str>>(stdout: &str, keys: &[S]) -> SmallMap<String, String> {
    bazel::server::info::parse_output(stdout, keys)
        .into_iter()
        .collect()
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
pub fn release_version(signals: &Arc<Signals>) -> Option<semver::Version> {
    RELEASE_VERSION
        .get_or_init(|| {
            Server::new(&SignalRunner(signals), &[])
                .info()
                .ok()
                .and_then(|(_pid, version)| version)
        })
        .clone()
}
