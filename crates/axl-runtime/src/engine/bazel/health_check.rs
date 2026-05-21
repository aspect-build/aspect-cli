use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

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
fn check_bazel_server(startup_flags: &[String]) -> CheckResult {
    let mut cmd = Command::new(super::bazel_binary());
    cmd.args(startup_flags)
        .arg("--noblock_for_lock")
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
fn get_output_base(startup_flags: &[String]) -> Option<PathBuf> {
    let mut cmd = Command::new(super::bazel_binary());
    cmd.args(startup_flags)
        .arg("info")
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

/// Directory entries that bazel can strand in `<output_base>/sandbox/`
/// when an invocation is SIGKILL'd mid-cleanup. The next command on the
/// same output_base then crashes in `SandboxBase.tidyUp` with
/// `"... is supposed to be moved, but file exists"`. See
/// bazelbuild/bazel#23880.
const STRANDED_SANDBOX_ENTRIES: &[&str] = &["_moved_trash_dir", "sandbox_stash"];

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
/// Logs each removed path so the cleanup is visible in CI output.
/// Returns the list of removed paths.
fn cleanup_stranded_sandbox_state(output_base: &Path) -> Vec<PathBuf> {
    let sandbox_base = output_base.join("sandbox");
    let mut removed = Vec::new();
    for name in STRANDED_SANDBOX_ENTRIES {
        let path = sandbox_base.join(name);
        // symlink_metadata so symlinks are inspected, not followed.
        let Ok(meta) = std::fs::symlink_metadata(&path) else {
            continue;
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
            Ok(()) => removed.push(path),
            Err(e) => tracing::warn!(
                path = %path.display(),
                error = %e,
                "Failed to remove stranded sandbox state — next invocation may still hit bazelbuild/bazel#23880",
            ),
        }
    }
    removed
}

pub fn run(startup_flags: &[String]) -> HealthCheckResult {
    // Step 1: Determine server directories
    let output_base = get_output_base(startup_flags);

    // Step 1.5: Clean up stranded sandbox state from any prior SIGKILL'd
    // invocation before bazel touches the sandbox in this command. See
    // `cleanup_stranded_sandbox_state` for context.
    if let Some(ref base) = output_base {
        let _ = cleanup_stranded_sandbox_state(base);
    }

    let server_pid_file = output_base
        .as_ref()
        .map(|base| base.join("server").join("server.pid.txt"));

    // Step 2: Run health check
    let result = check_bazel_server(startup_flags);

    // Step 3: Success
    if result.success {
        return HealthCheckResult {
            outcome: "healthy".to_string(),
            message: None,
            exit_code: Some(0),
        };
    }

    // Step 4: Failure
    let exit_code = result.exit_code;

    // 4a: Non-retryable error → inconclusive
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

    // 4b: Retryable error → attempt recovery
    let diagnostic = format!(
        "Bazel server returned an exit code ({}) that has caused the health check to fail",
        exit_code.map_or("unknown".to_string(), |c| c.to_string())
    );

    // 4b.i: Extract server PID from filesystem
    let pid = extract_server_pid(server_pid_file.as_deref());

    // 4b.ii: PID cannot be determined
    let Some(pid) = pid else {
        return HealthCheckResult {
            outcome: "unhealthy".to_string(),
            message: Some(diagnostic),
            exit_code,
        };
    };

    // 4b.iii / 4b.iv: Kill if running, then retry
    if super::process::is_pid_running(pid) {
        super::process::sigkill(pid);
    }

    // Retry health check
    let retry = check_bazel_server(startup_flags);

    if retry.success {
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
        let removed = cleanup_stranded_sandbox_state(base.path());
        assert!(removed.is_empty());
    }

    #[test]
    fn cleanup_removes_stranded_dirs() {
        let base = make_output_base();
        let sandbox = base.path().join("sandbox");
        let moved_trash = sandbox.join("_moved_trash_dir");
        let stash = sandbox.join("sandbox_stash");
        std::fs::create_dir(&moved_trash).unwrap();
        std::fs::create_dir(&stash).unwrap();
        // Non-empty dirs — exercise remove_dir_all.
        std::fs::write(moved_trash.join("leftover"), b"junk").unwrap();
        std::fs::write(stash.join("leftover"), b"junk").unwrap();

        let removed = cleanup_stranded_sandbox_state(base.path());
        assert_eq!(removed.len(), 2);
        assert!(!moved_trash.exists());
        assert!(!stash.exists());
    }

    #[test]
    fn cleanup_ignores_unrelated_entries() {
        let base = make_output_base();
        let sandbox = base.path().join("sandbox");
        let other = sandbox.join("linux-sandbox");
        std::fs::create_dir(&other).unwrap();

        let removed = cleanup_stranded_sandbox_state(base.path());
        assert!(removed.is_empty());
        assert!(other.exists(), "must not touch the per-strategy sandbox dirs");
    }

    #[test]
    fn cleanup_handles_missing_sandbox_dir() {
        // No sandbox subdirectory at all — e.g. fresh output_base.
        let base = tempfile::tempdir().expect("tempdir");
        let removed = cleanup_stranded_sandbox_state(base.path());
        assert!(removed.is_empty());
    }
}
