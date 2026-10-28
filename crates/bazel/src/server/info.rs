//! Reading what `bazel info` prints.

use std::io;

use indexmap::IndexMap;

/// Keys [`super::Server::info`] requests and then looks up in the parsed
/// output. Shared so the two sides can't drift — a lookup that misses the
/// requested spelling reads as "bazel didn't report it".
pub(super) const SERVER_PID_KEY: &str = "server_pid";
pub(super) const RELEASE_KEY: &str = "release";

/// Parse the value of `bazel info release` into a semver version.
///
/// The value looks like `release 9.0.0`, or `release 9.0.0-rc1` for a
/// release candidate. Non-release builds report a value with no version
/// number — `development version` (built from source) or `no_version` —
/// and return `None` rather than erroring, so a non-release Bazel doesn't
/// abort the task. Callers treat `None` as "version unknown".
///
/// The full version is preserved, **including** any pre-release suffix (so
/// `ctx.bazel.version(strip=False)` can surface `9.0.0-rc1`). Flag gating
/// ignores the suffix at the comparison site (see `constraint_matches`), so an
/// rc still matches the same constraints its release will.
pub fn parse_release(value: &str) -> Option<semver::Version> {
    let ver_str = value.trim().trim_start_matches("release ").trim();
    semver::Version::parse(ver_str).ok()
}

/// Parse `bazel info` stdout into a key/value map.
///
/// Bazel formats the output by how many keys were requested: exactly one key
/// prints the bare value, while zero (all keys) or two or more print
/// `key: value` lines. A single key is therefore taken from the request rather
/// than the output — parsing it as a pair would yield nothing, or split a value
/// that itself contains `": "` (e.g. a Windows path).
pub fn parse_output<S: AsRef<str>>(stdout: &str, keys: &[S]) -> IndexMap<String, String> {
    let mut map = IndexMap::new();
    if let [key] = keys {
        let value = stdout.trim();
        if !value.is_empty() {
            map.insert(key.as_ref().to_string(), value.to_string());
        }
        return map;
    }
    for line in stdout.lines() {
        if let Some((key, value)) = line.split_once(": ") {
            map.insert(key.trim().to_string(), value.trim().to_string());
        }
    }
    map
}

/// Pull `(server_pid, release)` out of `bazel info server_pid release` stdout.
///
/// A missing or unparseable `server_pid` is an error — callers need the pid.
/// A missing or non-release version is not: version-conditional flags fall
/// back to assuming latest.
pub(super) fn parse_server_info(stdout: &str) -> io::Result<(u32, Option<semver::Version>)> {
    let info = parse_output(stdout, &[SERVER_PID_KEY, RELEASE_KEY]);

    let version = info.get(RELEASE_KEY).and_then(|v| {
        let version = parse_release(v);
        if version.is_none() {
            // Logged for diagnosability when flag resolution looks off.
            tracing::debug!(
                release = %v,
                "bazel reported a non-release version; \
                 version-conditional flags will assume latest"
            );
        }
        version
    });

    let pid = info
        .get(SERVER_PID_KEY)
        .and_then(|v| v.parse::<u32>().ok())
        .ok_or_else(|| io::Error::other("bazel info did not return server_pid"))?;

    Ok((pid, version))
}

#[cfg(test)]
mod tests {
    use super::{parse_output, parse_release, parse_server_info};

    #[test]
    fn server_info_reads_pid_and_version() {
        let (pid, version) =
            parse_server_info("server_pid: 12345\nrelease: release 9.0.0\n").unwrap();
        assert_eq!(pid, 12345);
        assert_eq!(version, Some(semver::Version::new(9, 0, 0)));
    }

    #[test]
    fn server_info_tolerates_a_non_release_version() {
        let (pid, version) =
            parse_server_info("server_pid: 7\nrelease: development version\n").unwrap();
        assert_eq!(pid, 7);
        assert_eq!(version, None);
    }

    #[test]
    fn server_info_requires_a_parseable_pid() {
        for stdout in ["release: release 9.0.0\n", "server_pid: not-a-pid\n", ""] {
            let err = parse_server_info(stdout).unwrap_err();
            assert!(
                err.to_string().contains("did not return server_pid"),
                "unexpected error for {stdout:?}: {err}"
            );
        }
    }

    #[test]
    fn single_key_is_taken_from_the_request() {
        // `bazel info <key>` prints the bare value with no `key: ` prefix.
        let map = parse_output("/tmp/ws/bazel-out\n", &["output_path"]);
        assert_eq!(
            map.get("output_path").map(String::as_str),
            Some("/tmp/ws/bazel-out")
        );
    }

    #[test]
    fn single_key_value_containing_a_colon_is_not_split() {
        let map = parse_output("C:\\ws\\out: dir/command.log\n", &["command_log"]);
        assert_eq!(
            map.get("command_log").map(String::as_str),
            Some("C:\\ws\\out: dir/command.log")
        );
    }

    #[test]
    fn multiple_keys_are_parsed_as_pairs() {
        let map = parse_output(
            "output_path: /tmp/ws/bazel-out\noutput_base: /tmp/ws\n",
            &["output_path", "output_base"],
        );
        assert_eq!(
            map.get("output_path").map(String::as_str),
            Some("/tmp/ws/bazel-out")
        );
        assert_eq!(map.get("output_base").map(String::as_str), Some("/tmp/ws"));
    }

    #[test]
    fn no_keys_parses_every_pair() {
        let map = parse_output::<&str>(
            "bazel-bin: /tmp/ws/bin\noutput_path: /tmp/ws/bazel-out\n",
            &[],
        );
        assert_eq!(map.len(), 2);
        assert_eq!(
            map.get("bazel-bin").map(String::as_str),
            Some("/tmp/ws/bin")
        );
    }

    #[test]
    fn lines_without_a_pair_separator_are_skipped() {
        let map = parse_output::<&str>("a bare line\noutput_base: /tmp/ws\n", &[]);
        assert_eq!(map.len(), 1);
        assert_eq!(map.get("output_base").map(String::as_str), Some("/tmp/ws"));
    }

    #[test]
    fn empty_output_yields_no_entries() {
        assert!(parse_output("\n", &["output_path"]).is_empty());
        assert!(parse_output::<&str>("", &[]).is_empty());
    }

    #[test]
    fn parses_a_plain_release() {
        assert_eq!(
            parse_release("release 9.0.0"),
            Some(semver::Version::new(9, 0, 0))
        );
    }

    #[test]
    fn preserves_rc_and_pre_suffixes() {
        // The full version is kept (including the pre-release); gating strips it
        // at the comparison site, not here.
        assert_eq!(
            parse_release("release 9.0.0-rc1"),
            semver::Version::parse("9.0.0-rc1").ok()
        );
        assert_eq!(
            parse_release("release 8.0.0-pre.20251201.1"),
            semver::Version::parse("8.0.0-pre.20251201.1").ok()
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
