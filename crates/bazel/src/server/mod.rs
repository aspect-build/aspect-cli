//! One bazel server and what can be done to it.
//!
//! A server is identified by the startup flags a client is run with, which
//! decide its output base. Every question asked of it is a bazel client run
//! through a [`Runner`], so a host that tracks its children can stop them.

use std::io;
use std::process::Stdio;
use std::time::{Duration, Instant};

use crate::process::signal::{self, Target};
use crate::runner::Runner;

pub mod health;
pub mod info;
pub mod sandbox;

/// How long to wait for the Bazel client to exit after a second SIGINT
/// before escalating to SIGKILL.
const FORCE_KILL_TIMEOUT: Duration = Duration::from_millis(5000);
const FORCE_KILL_POLL: Duration = Duration::from_millis(100);

pub struct Server<'a> {
    runner: &'a dyn Runner,
    startup_flags: &'a [String],
}

impl<'a> Server<'a> {
    pub fn new(runner: &'a dyn Runner, startup_flags: &'a [String]) -> Self {
        Self {
            runner,
            startup_flags,
        }
    }

    /// `bazel info install_base` — the directory `@bazel_tools`
    /// (`embedded_tools`) is materialized under. Honors the startup flags
    /// (`--output_user_root` moves the install base).
    pub fn install_base(&self) -> io::Result<std::path::PathBuf> {
        let mut cmd = self.runner.command();
        cmd.args(self.startup_flags);
        cmd.arg("info");
        cmd.arg("--lockfile_mode=off");
        cmd.arg("install_base");
        cmd.stdout(Stdio::piped());
        cmd.stderr(Stdio::piped());
        cmd.stdin(Stdio::null());
        let output = self.runner.spawn(&mut cmd)?.wait_with_output()?;
        if !output.status.success() {
            return Err(io::Error::other(format!(
                "`bazel info install_base` failed while materializing aspects: {}",
                String::from_utf8_lossy(&output.stderr).trim()
            )));
        }
        Ok(std::path::PathBuf::from(
            String::from_utf8_lossy(&output.stdout).trim().to_string(),
        ))
    }

    /// `bazel <startup flags> --noblock_for_lock info --lockfile_mode=off`,
    /// the way every non-blocking probe starts.
    fn noblock_info(&self) -> std::process::Command {
        let mut cmd = self.runner.command();
        cmd.args(self.startup_flags);
        cmd.arg("--noblock_for_lock");
        cmd.arg("info");
        cmd.arg("--lockfile_mode=off");
        cmd
    }

    /// The server's pid and bazel's release version.
    ///
    /// The version is `None` when Bazel reports a non-release build (see
    /// [`info::parse_release`]); the pid is always required.
    pub fn info(&self) -> io::Result<(u32, Option<semver::Version>)> {
        let mut cmd = self.runner.command();
        cmd.args(self.startup_flags);
        cmd.arg("info");
        cmd.arg("--lockfile_mode=off");
        cmd.arg(info::SERVER_PID_KEY);
        cmd.arg(info::RELEASE_KEY);
        cmd.stdout(Stdio::piped());
        cmd.stderr(Stdio::piped());
        cmd.stdin(Stdio::null());
        // `bazel info` (without --noblock_for_lock) can hang on a busy
        // server. Spawned through the runner so a cancel reaches it, and so
        // the wait is a safe point.
        let c = self
            .runner
            .spawn(&mut cmd)
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
            return Err(io::Error::other(format!(
                "failed to determine Bazel server info ({detail})"
            )));
        }

        let stdout = String::from_utf8_lossy(&c.stdout);
        info::parse_server_info(&stdout)
    }

    /// The pid of the client holding the server's lock, by running
    /// `bazel --noblock_for_lock info`.
    ///
    /// When another invocation holds the lock, bazel exits with code 9 and prints:
    ///   "Another command (pid=12345) is running. Exiting immediately."
    /// We parse the PID from that stderr message.
    pub fn client_pid(&self) -> Option<u32> {
        let mut cmd = self.noblock_info();
        cmd.arg("server_pid");
        cmd.stdout(Stdio::null());
        cmd.stderr(Stdio::piped());
        cmd.stdin(Stdio::null());
        let output = self.runner.spawn(&mut cmd).ok()?.wait_with_output().ok()?;
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

    /// Whether the server's lock is currently held by a client.
    pub fn is_busy(&self) -> bool {
        let mut cmd = self.noblock_info();
        cmd.arg("server_pid");
        cmd.stdout(Stdio::null());
        cmd.stderr(Stdio::null());
        cmd.stdin(Stdio::null());
        let Ok(spawned) = self.runner.spawn(&mut cmd) else {
            return false;
        };
        matches!(spawned.wait_with_output(), Ok(o) if o.status.code() == Some(9))
    }

    /// The server's pid without blocking on the lock.
    ///
    /// Resolves `output_base` via `bazel --noblock_for_lock info output_base`
    /// (computed client-side, never blocks on the lock) and reads the PID from
    /// `<output_base>/server/server.pid.txt`.
    ///
    /// Returns `None` only if the server is not running or bazel is not available.
    pub fn pid_nonblocking(&self) -> Option<u32> {
        let mut cmd = self.noblock_info();
        cmd.arg("output_base");
        cmd.stdout(Stdio::piped());
        cmd.stderr(Stdio::null());
        cmd.stdin(Stdio::null());
        let output = self.runner.spawn(&mut cmd).ok()?.wait_with_output().ok()?;
        if !output.status.success() {
            return None;
        }
        let output_base = String::from_utf8_lossy(&output.stdout);
        let pid_path = std::path::Path::new(output_base.trim()).join("server/server.pid.txt");
        let contents = std::fs::read_to_string(pid_path).ok()?;
        contents.trim().parse::<u32>().ok()
    }

    /// Probe the server and recover from a held lock or a wedged server when
    /// possible; see [`health`]. `log` receives one line per step taken.
    pub fn health(&self, log: &mut dyn FnMut(&str)) -> health::HealthCheckResult {
        health::run(self.runner, self.startup_flags, log)
    }

    /// Cancel the running invocation forcefully, following Bazel's 3-SIGINT
    /// protocol. Whoever cancelled it already sent the 1st SIGINT (graceful
    /// cancel); this sends the 2nd and 3rd to escalate:
    ///   - 2nd SIGINT: repeated CancelRequest (Bazel is already cancelling)
    ///   - 3rd SIGINT: triggers Bazel's built-in KillServerProcess + client exit
    ///
    /// If the client still doesn't exit after the 3rd SIGINT, SIGKILL both the
    /// client and server as a last resort.
    ///
    /// Returns `true` if a signal was sent, `false` if neither the client nor
    /// the server could be found (the build may have already finished).
    pub fn force_kill(&self) -> bool {
        if let Some(client_pid) = self.client_pid() {
            // 2nd SIGINT: repeated cancel request.
            tracing::warn!(
                "cancel_invocation: sending 2nd SIGINT to Bazel client PID {client_pid}"
            );
            signal::interrupt(Target::Pid(client_pid));

            // 3rd SIGINT: triggers Bazel's built-in server kill + client exit.
            tracing::warn!(
                "cancel_invocation: sending 3rd SIGINT to Bazel client PID {client_pid}"
            );
            signal::interrupt(Target::Pid(client_pid));

            // Monitor the client — if it doesn't exit within the timeout,
            // SIGKILL both the client and the server ourselves.
            let start = Instant::now();
            while signal::is_running(Target::Pid(client_pid)) {
                if start.elapsed() >= FORCE_KILL_TIMEOUT {
                    tracing::warn!(
                        "cancel_invocation: Bazel client PID {client_pid} did not exit \
                         after {}ms, sending SIGKILL",
                        FORCE_KILL_TIMEOUT.as_millis()
                    );
                    signal::kill(Target::Pid(client_pid));
                    if let Some(server_pid) = self.pid_nonblocking() {
                        tracing::warn!(
                            "cancel_invocation: also sending SIGKILL to Bazel server PID \
                             {server_pid}"
                        );
                        signal::kill(Target::Pid(server_pid));
                    }
                    return true;
                }
                std::thread::sleep(FORCE_KILL_POLL);
            }
            return true;
        }

        // Client is gone (crashed or already exited). Only SIGKILL the server
        // if it's still busy — otherwise there's nothing to cancel.
        if self.is_busy()
            && let Some(pid) = self.pid_nonblocking()
        {
            tracing::warn!(
                "cancel_invocation: Bazel client not found, sending SIGKILL to \
                 server PID {pid}"
            );
            signal::kill(Target::Pid(pid));
            return true;
        }
        false
    }
}
