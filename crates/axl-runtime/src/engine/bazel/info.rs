use std::io;
use std::process::Command;
use std::process::Stdio;

use anyhow::anyhow;

/// Query bazel server info (server_pid, release version).
pub fn server_info() -> io::Result<(u32, semver::Version)> {
    server_info_with_startup_flags(&[])
}

/// Query bazel server info with startup flags prepended before the subcommand.
pub fn server_info_with_startup_flags(
    startup_flags: &[String],
) -> io::Result<(u32, semver::Version)> {
    let mut cmd = Command::new(super::bazel_binary());
    cmd.args(startup_flags);
    cmd.arg("info");
    cmd.arg("server_pid");
    cmd.arg("release");
    cmd.stdout(Stdio::piped());
    cmd.stderr(Stdio::piped());
    cmd.stdin(Stdio::null());
    let c = cmd
        .spawn()
        .map_err(|e| io::Error::other(format!("failed to spawn bazel: {e}")))?
        .wait_with_output()?;
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

    // When bazel info is called with multiple keys it emits "key: value" lines.
    let stdout = String::from_utf8_lossy(&c.stdout);
    let mut pid: Option<u32> = None;
    let mut version: Option<semver::Version> = None;
    for line in stdout.lines() {
        if let Some((key, value)) = line.split_once(": ") {
            match key.trim() {
                "server_pid" => {
                    pid = value.trim().parse::<u32>().ok();
                }
                "release" => {
                    // Value is like "release 9.0.0" or "release 9.0.0-rc1"
                    let ver_str = value.trim().trim_start_matches("release ").trim();
                    // Strip pre-release suffix: "9.0.0-rc1" -> "9.0.0"
                    let ver_str = ver_str.split('-').next().unwrap_or(ver_str);
                    version = semver::Version::parse(ver_str)
                        .map_err(|e| {
                            io::Error::other(anyhow!(
                                "failed to parse Bazel version '{}': {}",
                                ver_str,
                                e
                            ))
                        })
                        .ok();
                }
                _ => {}
            }
        }
    }

    let pid =
        pid.ok_or_else(|| io::Error::other(anyhow!("bazel info did not return server_pid")))?;
    let version = version.ok_or_else(|| {
        io::Error::other(anyhow!(
            "bazel info did not return a parseable release version"
        ))
    })?;

    Ok((pid, version))
}

/// Determine the real bazel client PID by running `bazel --noblock_for_lock info`.
///
/// When another invocation holds the lock, bazel exits with code 9 and prints:
///   "Another command (pid=12345) is running. Exiting immediately."
/// We parse the PID from that stderr message.
pub fn client_pid(startup_flags: &[String]) -> Option<u32> {
    let mut cmd = Command::new(super::bazel_binary());
    cmd.args(startup_flags);
    cmd.arg("--noblock_for_lock");
    cmd.arg("info");
    cmd.arg("server_pid");
    cmd.stdout(Stdio::null());
    cmd.stderr(Stdio::piped());
    cmd.stdin(Stdio::null());
    let output = cmd.output().ok()?;
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
    let mut cmd = Command::new(super::bazel_binary());
    cmd.args(startup_flags);
    cmd.arg("--noblock_for_lock");
    cmd.arg("info");
    cmd.arg("server_pid");
    cmd.stdout(Stdio::null());
    cmd.stderr(Stdio::null());
    cmd.stdin(Stdio::null());
    matches!(cmd.output(), Ok(o) if o.status.code() == Some(9))
}

/// Query the server PID without blocking on the lock.
///
/// Resolves `output_base` via `bazel --noblock_for_lock info output_base`
/// (computed client-side, never blocks on the lock) and reads the PID from
/// `<output_base>/server/server.pid.txt`.
///
/// Returns `None` only if the server is not running or bazel is not available.
pub fn server_pid_nonblocking(startup_flags: &[String]) -> Option<u32> {
    let mut cmd = Command::new(super::bazel_binary());
    cmd.args(startup_flags);
    cmd.arg("--noblock_for_lock");
    cmd.arg("info");
    cmd.arg("output_base");
    cmd.stdout(Stdio::piped());
    cmd.stderr(Stdio::null());
    cmd.stdin(Stdio::null());
    let output = cmd.output().ok()?;
    if !output.status.success() {
        return None;
    }
    let output_base = String::from_utf8_lossy(&output.stdout);
    let pid_path = std::path::Path::new(output_base.trim()).join("server/server.pid.txt");
    let contents = std::fs::read_to_string(pid_path).ok()?;
    contents.trim().parse::<u32>().ok()
}
