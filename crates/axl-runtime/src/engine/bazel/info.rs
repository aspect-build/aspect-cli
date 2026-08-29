use std::io;
use std::process::Stdio;
use std::sync::OnceLock;

use anyhow::anyhow;
use starlark::collections::SmallMap;

/// Keys [`server_info`] requests and then looks up in the parsed output. Shared
/// so the two sides can't drift — a lookup that misses the requested spelling
/// reads as "bazel didn't report it".
const SERVER_PID_KEY: &str = "server_pid";
const RELEASE_KEY: &str = "release";

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
fn parse_release(value: &str) -> Option<semver::Version> {
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
pub fn parse_output<S: AsRef<str>>(stdout: &str, keys: &[S]) -> SmallMap<String, String> {
    let mut map = SmallMap::new();
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

/// Query bazel server info (server_pid, release version).
///
/// The version is `None` when Bazel reports a non-release build (see
/// [`parse_release`]); the pid is always required.
pub fn server_info() -> io::Result<(u32, Option<semver::Version>)> {
    server_info_with_startup_flags(&[])
}

/// Query bazel server info with startup flags prepended before the subcommand.
pub fn server_info_with_startup_flags(
    startup_flags: &[String],
) -> io::Result<(u32, Option<semver::Version>)> {
    let mut cmd = super::bazel_command();
    cmd.args(startup_flags);
    cmd.arg("info");
    cmd.arg(SERVER_PID_KEY);
    cmd.arg(RELEASE_KEY);
    cmd.stdout(Stdio::piped());
    cmd.stderr(Stdio::piped());
    cmd.stdin(Stdio::null());
    // `bazel info` (without --noblock_for_lock) can hang on a busy
    // server. Register so the OS signal handler can SIGINT it on
    // CI-cancel.
    let (child, _guard) = super::live::spawn_registered(&mut cmd)
        .map_err(|e| io::Error::other(format!("failed to spawn bazel: {e}")))?;
    let c = child.wait_with_output()?;
    if !c.status.success() {
        let stderr = String::from_utf8_lossy(&c.stderr);
        let stderr = stderr.trim();
        let detail = if stderr.is_empty() {
            format!("exit code {:?}", c.status.code())
        } else {
            format!("exit code {:?}: {}", c.status.code(), stderr)
        };
        return Err(io::Error::other(anyhow!(
            "failed to determine Bazel server info ({})",
            detail
        )));
    }

    let stdout = String::from_utf8_lossy(&c.stdout);
    parse_server_info(&stdout)
}

/// Pull `(server_pid, release)` out of `bazel info server_pid release` stdout.
///
/// A missing or unparseable `server_pid` is an error — callers need the pid.
/// A missing or non-release version is not: version-conditional flags fall
/// back to assuming latest.
fn parse_server_info(stdout: &str) -> io::Result<(u32, Option<semver::Version>)> {
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
        .ok_or_else(|| io::Error::other(anyhow!("bazel info did not return server_pid")))?;

    Ok((pid, version))
}

/// Process-wide cache of the Bazel release version, populated on first use.
static RELEASE_VERSION: OnceLock<Option<semver::Version>> = OnceLock::new();

/// The Bazel release version, probed once per process via [`server_info`] and
/// memoized. `None` when Bazel reports a non-release build or the probe fails —
/// callers treat that as "version unknown" (assume latest for version-gated
/// flags).
///
/// The version is stable for the lifetime of an `aspect` invocation, so the
/// single probe is shared by every caller (flag-gating, rc-section selection,
/// …) instead of each shelling out to Bazel independently.
pub fn release_version() -> Option<semver::Version> {
    RELEASE_VERSION
        .get_or_init(|| server_info().ok().and_then(|(_pid, version)| version))
        .clone()
}

/// Determine the real bazel client PID by running `bazel --noblock_for_lock info`.
///
/// When another invocation holds the lock, bazel exits with code 9 and prints:
///   "Another command (pid=12345) is running. Exiting immediately."
/// We parse the PID from that stderr message.
pub fn client_pid(startup_flags: &[String]) -> Option<u32> {
    let mut cmd = super::bazel_command();
    cmd.args(startup_flags);
    cmd.arg("--noblock_for_lock");
    cmd.arg("info");
    cmd.arg("server_pid");
    cmd.stdout(Stdio::null());
    cmd.stderr(Stdio::piped());
    cmd.stdin(Stdio::null());
    let (child, _guard) = super::live::spawn_registered(&mut cmd).ok()?;
    let output = child.wait_with_output().ok()?;
    // Exit code 9 means the lock is held — stderr contains the client PID.
    if output.status.code() != Some(9) {
        return None;
    }
    let stderr = String::from_utf8_lossy(&output.stderr);
    // Parse "Another command (pid=12345) is running."
    let start = stderr.find("pid=")? + 4;
    let rest = &stderr[start..];
    let end = rest.find(')')?;
    rest[..end].parse::<u32>().ok()
}

/// Check if the bazel server lock is currently held by a client.
pub fn is_server_busy(startup_flags: &[String]) -> bool {
    let mut cmd = super::bazel_command();
    cmd.args(startup_flags);
    cmd.arg("--noblock_for_lock");
    cmd.arg("info");
    cmd.arg("server_pid");
    cmd.stdout(Stdio::null());
    cmd.stderr(Stdio::null());
    cmd.stdin(Stdio::null());
    let Ok((child, _guard)) = super::live::spawn_registered(&mut cmd) else {
        return false;
    };
    matches!(child.wait_with_output(), Ok(o) if o.status.code() == Some(9))
}

/// Query the server PID without blocking on the lock.
///
/// Resolves `output_base` via `bazel --noblock_for_lock info output_base`
/// (computed client-side, never blocks on the lock) and reads the PID from
/// `<output_base>/server/server.pid.txt`.
///
/// Returns `None` only if the server is not running or bazel is not available.
pub fn server_pid_nonblocking(startup_flags: &[String]) -> Option<u32> {
    let mut cmd = super::bazel_command();
    cmd.args(startup_flags);
    cmd.arg("--noblock_for_lock");
    cmd.arg("info");
    cmd.arg("output_base");
    cmd.stdout(Stdio::piped());
    cmd.stderr(Stdio::null());
    cmd.stdin(Stdio::null());
    let (child, _guard) = super::live::spawn_registered(&mut cmd).ok()?;
    let output = child.wait_with_output().ok()?;
    if !output.status.success() {
        return None;
    }
    let output_base = String::from_utf8_lossy(&output.stdout);
    let pid_path = std::path::Path::new(output_base.trim()).join("server/server.pid.txt");
    let contents = std::fs::read_to_string(pid_path).ok()?;
    contents.trim().parse::<u32>().ok()
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
