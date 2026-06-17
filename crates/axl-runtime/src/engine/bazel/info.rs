//! Parsing helpers for `bazel info` output. The spawning lives on
//! [`super::backend::BazelBackend`] (so it picks Real vs Fake); this module
//! only turns bazel's text output into typed values.

use std::collections::BTreeMap;

/// Parse `bazel info` `key: value` lines into a map. Splits on the first
/// `": "` so values containing colons are preserved.
pub(super) fn parse_info_map(stdout: &str) -> BTreeMap<String, String> {
    let mut map = BTreeMap::new();
    for line in stdout.lines() {
        if let Some((key, value)) = line.split_once(": ") {
            map.insert(key.trim().to_string(), value.trim().to_string());
        }
    }
    map
}

/// Parse the value of `bazel info release` into a semver version.
///
/// The value looks like `release 9.0.0`, or `release 9.0.0-rc1` for a
/// release candidate. Non-release builds report a value with no version
/// number — `development version` (built from source) or `no_version` —
/// and return `None` rather than erroring, so a non-release Bazel doesn't
/// abort the task. Callers treat `None` as "version unknown".
pub(super) fn parse_release(value: &str) -> Option<semver::Version> {
    let ver_str = value.trim().trim_start_matches("release ").trim();
    // Drop any pre-release suffix so an rc/pre build (`9.0.0-rc1`) matches the
    // same constraints its eventual release will.
    let ver_str = ver_str.split('-').next().unwrap_or(ver_str);
    semver::Version::parse(ver_str).ok()
}

#[cfg(test)]
mod tests {
    use super::parse_release;

    #[test]
    fn parses_a_plain_release() {
        assert_eq!(
            parse_release("release 9.0.0"),
            Some(semver::Version::new(9, 0, 0))
        );
    }

    #[test]
    fn strips_rc_and_pre_suffixes() {
        assert_eq!(
            parse_release("release 9.0.0-rc1"),
            Some(semver::Version::new(9, 0, 0))
        );
        assert_eq!(
            parse_release("release 8.0.0-pre.20251201.1"),
            Some(semver::Version::new(8, 0, 0))
        );
    }

    #[test]
    fn non_release_builds_have_no_version() {
        assert_eq!(parse_release("development version"), None);
        assert_eq!(parse_release("no_version"), None);
        assert_eq!(parse_release(""), None);
        assert_eq!(parse_release("   "), None);
    }
}
