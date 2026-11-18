//! The release of the Bazel binary itself, from `bazel --version`, which the
//! client answers alone: asking starts, reaches and restarts no server,
//! whatever startup flags an invocation goes on to use. Also how a Bazel
//! version string reads as semver, for `bazel info release` too.

use std::io;
use std::process::Stdio;

use crate::runner::Runner;

/// The release of the Bazel binary `runner` runs; `Ok(None)` for a non-release
/// build. `--version` must be the only argument, as Bazel takes it only alone.
pub fn client_version(runner: &dyn Runner) -> io::Result<Option<semver::Version>> {
    let mut cmd = runner.command();
    cmd.arg("--version");
    cmd.stdout(Stdio::piped());
    cmd.stderr(Stdio::piped());
    cmd.stdin(Stdio::null());
    let out = runner
        .spawn(&mut cmd)
        .map_err(|e| io::Error::other(format!("failed to spawn bazel: {e}")))?
        .wait_with_output()?;
    if !out.status.success() {
        return Err(io::Error::other(format!(
            "`bazel --version` failed (exit code {:?}): {}",
            out.status.code(),
            String::from_utf8_lossy(&out.stderr).trim()
        )));
    }
    Ok(parse_version_output(&String::from_utf8_lossy(&out.stdout)))
}

/// The version in `bazel --version` stdout: the last line that reads
/// `bazel <version>` (`bazel 9.0.0`, `bazel 10.0.0rc3`), so a wrapper that
/// prints first is read past. `None` for a non-release build (`bazel
/// no_version`, `bazel development version`) or no such line.
pub fn parse_version_output(stdout: &str) -> Option<semver::Version> {
    stdout
        .lines()
        .rev()
        .find_map(|line| parse(line.trim().strip_prefix("bazel ")?))
}

/// A Bazel version string as semver, the pre-release suffix kept: any semver
/// as it is (a rolling release's `9.0.0-pre.20250101.1`), and Bazel's
/// spelling of a release candidate, `10.0.0rc3`, as `10.0.0-rc3`. `None`
/// otherwise.
pub fn parse(version: &str) -> Option<semver::Version> {
    let version = version.trim();
    if let Ok(v) = semver::Version::parse(version) {
        return Some(v);
    }
    let core = version.find(|c: char| !(c.is_ascii_digit() || c == '.'))?;
    if core == 0 || !version[core..].starts_with("rc") {
        return None;
    }
    semver::Version::parse(&format!("{}-{}", &version[..core], &version[core..])).ok()
}

#[cfg(test)]
mod tests {
    use super::{parse, parse_version_output};

    fn version(v: &str) -> Option<semver::Version> {
        semver::Version::parse(v).ok()
    }

    #[test]
    fn a_release_and_a_candidate_are_read() {
        assert_eq!(parse_version_output("bazel 8.1.0\n"), version("8.1.0"));
        assert_eq!(
            parse_version_output("bazel 10.0.0rc3"),
            version("10.0.0-rc3")
        );
    }

    #[test]
    fn a_wrapper_printing_first_is_read_past() {
        assert_eq!(
            parse_version_output("hello from tools/bazel\nbazel 7.1.0\n"),
            version("7.1.0")
        );
    }

    #[test]
    fn a_non_release_build_or_no_version_line_has_none() {
        assert_eq!(parse_version_output("bazel no_version"), None);
        assert_eq!(parse_version_output("bazel development version"), None);
        assert_eq!(parse_version_output("aspect 2026.1.0"), None);
        assert_eq!(parse_version_output(""), None);
    }

    #[test]
    fn bazels_version_spellings_read_as_semver() {
        assert_eq!(parse("9.3.2"), version("9.3.2"));
        assert_eq!(parse("10.0.0rc3"), version("10.0.0-rc3"));
        assert_eq!(
            parse("9.0.0-pre.20250101.1"),
            version("9.0.0-pre.20250101.1")
        );
        assert_eq!(parse("8.0.0rc"), version("8.0.0-rc"));
        assert_eq!(parse("7.4.0-homebrew"), version("7.4.0-homebrew"));
        assert_eq!(parse("development version"), None);
        assert_eq!(parse("no_version"), None);
        assert_eq!(parse("rc1"), None);
        assert_eq!(parse("v8.0.0"), None);
        assert_eq!(parse("8.0rc1"), None);
    }
}
