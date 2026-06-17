use std::path::{Path, PathBuf};
use std::process::Stdio;

use allocative::Allocative;
use derive_more::Display;
use starlark::environment::{Methods, MethodsBuilder, MethodsStatic};
use starlark::starlark_module;
use starlark::starlark_simple_value;
use starlark::values;
use starlark::values::none::NoneOr;
use starlark::values::starlark_value;
use starlark::values::{NoSerialize, ProvidesStaticType, ValueLike};

/// Bazel exit codes that indicate a potentially recoverable server issue.
const RETRYABLE_EXIT_CODES: &[i32] = &[
    1,  // Build or parsing failure
    37, // Blaze internal error
    36, // Local environmental error
    9,  // Lock held (noblock_for_lock)
];

#[derive(Debug, Display, ProvidesStaticType, NoSerialize, Allocative)]
#[display("<bazel.HealthCheckResult>")]
pub struct HealthCheckResult {
    /// One of "healthy", "unhealthy", or "inconclusive".
    outcome: String,
    message: Option<String>,
    exit_code: Option<i32>,
}

starlark_simple_value!(HealthCheckResult);

#[starlark_value(type = "bazel.HealthCheckResult")]
impl<'v> values::StarlarkValue<'v> for HealthCheckResult {
    fn get_methods() -> Option<&'static Methods> {
        static RES: MethodsStatic = MethodsStatic::new();
        RES.methods(health_check_result_methods)
    }
}

#[starlark_module]
pub(crate) fn health_check_result_methods(registry: &mut MethodsBuilder) {
    /// The server health state: `"healthy"`, `"unhealthy"`, or `"inconclusive"`.
    #[starlark(attribute)]
    fn outcome<'v>(this: values::Value<'v>) -> anyhow::Result<String> {
        Ok(this
            .downcast_ref::<HealthCheckResult>()
            .unwrap()
            .outcome
            .clone())
    }

    /// Diagnostic message, if any.
    #[starlark(attribute)]
    fn message<'v>(this: values::Value<'v>) -> anyhow::Result<NoneOr<String>> {
        Ok(NoneOr::from_option(
            this.downcast_ref::<HealthCheckResult>()
                .unwrap()
                .message
                .clone(),
        ))
    }

    /// The original Bazel exit code, if available.
    #[starlark(attribute)]
    fn exit_code<'v>(this: values::Value<'v>) -> anyhow::Result<NoneOr<i32>> {
        Ok(NoneOr::from_option(
            this.downcast_ref::<HealthCheckResult>().unwrap().exit_code,
        ))
    }
}

struct CheckResult {
    success: bool,
    exit_code: Option<i32>,
    stderr: String,
}

/// Runs `bazel [startup_flags] --noblock_for_lock info server_pid` and returns the result.
fn check_bazel_server(
    backend: &super::backend::BazelBackend,
    startup_flags: &[String],
) -> CheckResult {
    let mut cmd = backend.base_command(startup_flags);
    cmd.arg("--noblock_for_lock")
        .arg("info")
        .arg("server_pid")
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .stdin(Stdio::null());
    let output = match super::live::spawn_registered(&mut cmd) {
        Ok((child, _guard)) => child.wait_with_output(),
        Err(e) => Err(e),
    };

    match output {
        Ok(output) => CheckResult {
            success: output.status.success(),
            exit_code: output.status.code(),
            stderr: String::from_utf8_lossy(&output.stderr).to_string(),
        },
        Err(e) => CheckResult {
            success: false,
            exit_code: None,
            stderr: e.to_string(),
        },
    }
}

/// Reads the PID from a server PID file on disk.
///
/// Returns `None` if the path is not absolute, does not exist, cannot be read,
/// or does not contain a valid integer. The file is read as latin1 and trimmed.
fn extract_server_pid(server_pid_file: Option<&Path>) -> Option<u32> {
    let path = server_pid_file?;

    if !path.is_absolute() {
        return None;
    }

    let content = std::fs::read(path).ok()?;
    // latin1: each byte maps directly to a unicode codepoint
    let text: String = content.iter().map(|&b| b as char).collect();
    text.trim().parse::<u32>().ok()
}

/// Tries to determine the Bazel output base by running `bazel [startup_flags] info output_base`.
fn get_output_base(
    backend: &super::backend::BazelBackend,
    startup_flags: &[String],
) -> Option<PathBuf> {
    let mut cmd = backend.base_command(startup_flags);
    cmd.arg("info")
        .arg("output_base")
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .stdin(Stdio::null());
    let (child, _guard) = super::live::spawn_registered(&mut cmd).ok()?;
    let output = child.wait_with_output().ok()?;

    if !output.status.success() {
        return None;
    }

    let path = String::from_utf8_lossy(&output.stdout).trim().to_string();
    if path.is_empty() {
        return None;
    }
    Some(PathBuf::from(path))
}

/// Directory `<output_base>/sandbox/_moved_trash_dir` is the rename
/// target that `SandboxBase.tidyUp` moves the live sandbox base into
/// before async-deletion. If a prior invocation was SIGKILL'd between
/// the rename and the deletion, the directory is left behind, and the
/// next command on the same output_base aborts in `SandboxBase.tidyUp`
/// with `"... is supposed to be moved, but file exists"`. See
/// bazelbuild/bazel#23880.
///
/// Deliberately excludes `sandbox_stash` — that is the persistent
/// cross-invocation cache used by `--reuse_sandbox_directories` (on by
/// default in this repo's `bazel/defaults.bazelrc`), and its presence is
/// expected on a healthy runner. Wiping it on every health check would
/// silently undo sandbox reuse on every job.
const STRANDED_MOVED_TRASH_DIR: &str = "_moved_trash_dir";

/// Removes stranded sandbox state left by a previous invocation that
/// was SIGKILL'd before sandbox cleanup could finish. Without this, the
/// next bazel command on the same output_base aborts with the bug
/// described in bazelbuild/bazel#23880.
///
/// `aspect-cli/src/main.rs` already gives bazel a 5s SIGINT grace
/// window before escalating to SIGKILL, but on a heavily-loaded runner
/// cleanup can still time out — this is the safety net that lets the
/// next job on the runner proceed instead of hard-failing in
/// `afterCommand`.
///
/// Logs the removal so it is visible in CI output. Returns `true` iff
/// the entry was present and removed.
fn cleanup_stranded_sandbox_state(output_base: &Path) -> bool {
    let path = output_base.join("sandbox").join(STRANDED_MOVED_TRASH_DIR);
    // symlink_metadata so symlinks are inspected, not followed.
    let Ok(meta) = std::fs::symlink_metadata(&path) else {
        return false;
    };
    tracing::warn!(
        path = %path.display(),
        "Removing stranded sandbox state from a previous SIGKILL'd \
         invocation (bazelbuild/bazel#23880)"
    );
    let res = if meta.file_type().is_dir() {
        std::fs::remove_dir_all(&path)
    } else {
        std::fs::remove_file(&path)
    };
    match res {
        Ok(()) => true,
        Err(e) => {
            tracing::warn!(
                path = %path.display(),
                error = %e,
                "Failed to remove stranded sandbox state — next invocation may still hit bazelbuild/bazel#23880",
            );
            false
        }
    }
}

/// Extract `--output_base=<path>` from the passed startup flags without
/// invoking bazel. Returns `None` when no `--output_base` flag is present
/// or its value is empty.
///
/// Used in failure-path recovery where we can't safely run `bazel info
/// output_base` — the server is wedged holding the workspace lock, and
/// `bazel info` (without `--noblock_for_lock`) would queue behind it.
fn output_base_from_flags(startup_flags: &[String]) -> Option<PathBuf> {
    for flag in startup_flags {
        if let Some(value) = flag.strip_prefix("--output_base=") {
            if !value.is_empty() {
                return Some(PathBuf::from(value));
            }
        }
    }
    None
}

/// Probe the Bazel server and recover from a wedged-lock state when possible.
///
/// Outcomes:
///   - `healthy` — `--noblock_for_lock info server_pid` returned 0 (either
///     on the first try or after the recovery SIGKILL + retry).
///   - `inconclusive` — non-retryable error (likely a configuration issue).
///   - `unhealthy` — retryable failure we couldn't recover from (PID file
///     missing, or retry probe still fails after SIGKILL).
///
/// The `--noblock_for_lock` probe is intentionally the FIRST bazel call.
/// Any other invocation (in particular `bazel info output_base`) lacks the
/// flag and would queue behind a wedged server holding the workspace lock
/// — defeating the entire purpose of the health check. On the recovery
/// path we extract `--output_base` from the passed startup flags rather
/// than asking bazel, for the same reason.
///
/// On success, best-effort cleans up stranded sandbox state from a prior
/// SIGKILL'd invocation (bazelbuild/bazel#23880) before the next bazel
/// command runs.
pub fn run(backend: &super::backend::BazelBackend, startup_flags: &[String]) -> HealthCheckResult {
    let result = check_bazel_server(backend, startup_flags);

    if result.success {
        if let Some(base) = get_output_base(backend, startup_flags) {
            let _ = cleanup_stranded_sandbox_state(&base);
        }
        return HealthCheckResult {
            outcome: "healthy".to_string(),
            message: None,
            exit_code: Some(0),
        };
    }

    let exit_code = result.exit_code;

    if let Some(code) = exit_code {
        if !RETRYABLE_EXIT_CODES.contains(&code) {
            return HealthCheckResult {
                outcome: "inconclusive".to_string(),
                message: Some(format!(
                    "Unable to health check bazel server due to potential configuration issues: {}",
                    result.stderr.trim()
                )),
                exit_code: Some(code),
            };
        }
    }

    // Retryable failure: the server is wedged holding the workspace lock.
    // Find its PID via the on-disk PID file (asking bazel would block),
    // SIGKILL it, and retry the noblock probe.
    let diagnostic = format!(
        "Bazel server returned an exit code ({}) that has caused the health check to fail",
        exit_code.map_or("unknown".to_string(), |c| c.to_string())
    );

    let Some(output_base) = output_base_from_flags(startup_flags) else {
        return HealthCheckResult {
            outcome: "unhealthy".to_string(),
            message: Some(diagnostic),
            exit_code,
        };
    };

    let server_pid_file = output_base.join("server").join("server.pid.txt");
    let Some(pid) = extract_server_pid(Some(&server_pid_file)) else {
        return HealthCheckResult {
            outcome: "unhealthy".to_string(),
            message: Some(diagnostic),
            exit_code,
        };
    };

    if super::process::is_pid_running(pid) {
        super::process::sigkill(pid);
    }

    let retry = check_bazel_server(backend, startup_flags);

    if retry.success {
        let _ = cleanup_stranded_sandbox_state(&output_base);
        HealthCheckResult {
            outcome: "healthy".to_string(),
            message: None,
            exit_code: Some(0),
        }
    } else {
        HealthCheckResult {
            outcome: "unhealthy".to_string(),
            message: Some(diagnostic),
            exit_code,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_output_base() -> tempfile::TempDir {
        let dir = tempfile::tempdir().expect("tempdir");
        std::fs::create_dir(dir.path().join("sandbox")).expect("create sandbox dir");
        dir
    }

    #[test]
    fn cleanup_noop_when_sandbox_clean() {
        let base = make_output_base();
        assert!(!cleanup_stranded_sandbox_state(base.path()));
    }

    #[test]
    fn cleanup_removes_moved_trash_dir() {
        let base = make_output_base();
        let moved_trash = base.path().join("sandbox").join("_moved_trash_dir");
        std::fs::create_dir(&moved_trash).unwrap();
        // Non-empty dir — exercise remove_dir_all.
        std::fs::write(moved_trash.join("leftover"), b"junk").unwrap();

        assert!(cleanup_stranded_sandbox_state(base.path()));
        assert!(!moved_trash.exists());
    }

    #[test]
    fn cleanup_preserves_sandbox_stash() {
        // `sandbox_stash` is the persistent --reuse_sandbox_directories
        // cache; the health check must NOT touch it.
        let base = make_output_base();
        let stash = base.path().join("sandbox").join("sandbox_stash");
        std::fs::create_dir(&stash).unwrap();
        std::fs::write(stash.join("cached_action"), b"reuse-me").unwrap();

        assert!(!cleanup_stranded_sandbox_state(base.path()));
        assert!(
            stash.exists(),
            "sandbox_stash must survive the health check"
        );
    }

    #[test]
    fn cleanup_ignores_unrelated_entries() {
        let base = make_output_base();
        let other = base.path().join("sandbox").join("linux-sandbox");
        std::fs::create_dir(&other).unwrap();

        assert!(!cleanup_stranded_sandbox_state(base.path()));
        assert!(
            other.exists(),
            "must not touch the per-strategy sandbox dirs"
        );
    }

    #[test]
    fn cleanup_handles_missing_sandbox_dir() {
        // No sandbox subdirectory at all — e.g. fresh output_base.
        let base = tempfile::tempdir().expect("tempdir");
        assert!(!cleanup_stranded_sandbox_state(base.path()));
    }

    #[test]
    fn output_base_from_flags_finds_explicit_flag() {
        let flags = vec![
            "--nohome_rc".to_string(),
            "--output_base=/mnt/ephemeral/output/repo".to_string(),
            "--nosystem_rc".to_string(),
        ];
        assert_eq!(
            output_base_from_flags(&flags),
            Some(PathBuf::from("/mnt/ephemeral/output/repo"))
        );
    }

    #[test]
    fn output_base_from_flags_absent_returns_none() {
        let flags = vec![
            "--nohome_rc".to_string(),
            "--output_user_root=/mnt/ephemeral/bazel".to_string(),
        ];
        assert_eq!(output_base_from_flags(&flags), None);
    }

    #[test]
    fn output_base_from_flags_empty_value_returns_none() {
        // Defensive: malformed `--output_base=` should not silently
        // produce PathBuf::from("") which would join into nonsense.
        let flags = vec!["--output_base=".to_string()];
        assert_eq!(output_base_from_flags(&flags), None);
    }

    #[test]
    fn output_base_from_flags_prefix_match_only() {
        // `--output_user_root` shares a `--output_` prefix; must not
        // accidentally match.
        let flags = vec!["--output_user_root=/mnt/foo".to_string()];
        assert_eq!(output_base_from_flags(&flags), None);
    }
}
