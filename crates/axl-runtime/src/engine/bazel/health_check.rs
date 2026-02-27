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

/// Runs `bazel --noblock_for_lock info server_pid` and returns the result.
fn check_bazel_server() -> CheckResult {
    let output = Command::new("bazel")
        .arg("--noblock_for_lock")
        .arg("info")
        .arg("server_pid")
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .stdin(Stdio::null())
        .output();

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

/// Probes whether a process with the given PID exists using signal 0.
#[cfg(unix)]
fn is_pid_running(pid: u32) -> bool {
    use nix::sys::signal;
    use nix::unistd::Pid;

    signal::kill(Pid::from_raw(pid as i32), None).is_ok()
}

#[cfg(not(unix))]
fn is_pid_running(_pid: u32) -> bool {
    false
}

/// Sends SIGKILL to the given PID. Silently ignores failures.
#[cfg(unix)]
fn kill_server_pid(pid: u32) {
    use nix::sys::signal::{self, Signal};
    use nix::unistd::Pid;

    tracing::warn!("Workflows killing bazel server with PID {}", pid);
    let _ = signal::kill(Pid::from_raw(pid as i32), Signal::SIGKILL);
}

#[cfg(not(unix))]
fn kill_server_pid(_pid: u32) {
    tracing::warn!("kill_server_pid is not supported on this platform");
}

/// Tries to determine the Bazel output base by running `bazel info output_base`.
fn get_output_base() -> Option<PathBuf> {
    let output = Command::new("bazel")
        .arg("info")
        .arg("output_base")
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .stdin(Stdio::null())
        .output()
        .ok()?;

    if !output.status.success() {
        return None;
    }

    let path = String::from_utf8_lossy(&output.stdout).trim().to_string();
    if path.is_empty() {
        return None;
    }
    Some(PathBuf::from(path))
}

pub fn run(output_base: Option<&str>) -> HealthCheckResult {
    // Step 1: Determine server directories
    let output_base = match output_base {
        Some(path) => Some(PathBuf::from(path)),
        None => get_output_base(),
    };

    let server_pid_file = output_base
        .as_ref()
        .map(|base| base.join("server").join("server.pid.txt"));

    // Step 2: Run health check
    let result = check_bazel_server();

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
    if is_pid_running(pid) {
        kill_server_pid(pid);
    }

    // Retry health check
    let retry = check_bazel_server();

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
