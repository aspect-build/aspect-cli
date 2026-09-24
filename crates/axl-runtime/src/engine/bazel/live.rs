//! Live bazel clients used to forward shutdown signals.

use std::sync::OnceLock;

use crate::engine::process::{self, ProcessGuard, ProcessSupervisor};

fn supervisor() -> &'static ProcessSupervisor {
    static SUPERVISOR: OnceLock<ProcessSupervisor> = OnceLock::new();
    SUPERVISOR.get_or_init(ProcessSupervisor::default)
}

pub type LiveBazelGuard = ProcessGuard;

/// Spawns and tracks a bazel client until the returned guard is dropped.
pub fn spawn_registered(
    cmd: &mut std::process::Command,
) -> std::io::Result<(std::process::Child, LiveBazelGuard)> {
    supervisor().spawn(cmd, false)
}

/// Prevents new bazel clients from spawning during process shutdown.
pub fn begin_shutdown() {
    supervisor().begin_shutdown();
}

/// Returns a snapshot of registered bazel client PIDs.
pub fn live_pids() -> Vec<u32> {
    supervisor().live_pids()
}

/// Best-effort SIGINT to every registered bazel client. Non-blocking
/// — this is meant to be called from a signal handler that has very
/// little time to do work before forced exit. Idempotent — safe to
/// call multiple times in succession to mimic bazel's 3-SIGINT
/// cancel protocol (see [`bazel cancellation docs][1]):
///
///   1st SIGINT → graceful cancel of the in-flight command
///   2nd SIGINT → still graceful; gives a short window for cleanup
///   3rd SIGINT → triggers bazel's `KillServerProcess` and hard exit
///
/// [1]: https://bazel.build/run/cancellation
pub fn signal_all_for_shutdown() {
    for pid in live_pids() {
        if process::sigint(pid) {
            tracing::warn!("sent SIGINT to live bazel client PID {pid}");
        }
    }
}

/// SIGKILLs registered bazel clients that are still running.
pub fn force_kill_all_remaining() -> usize {
    let mut killed = 0;
    for pid in live_pids() {
        if process::sigkill(pid) {
            tracing::warn!("sent SIGKILL to bazel client PID {pid} after SIGINT grace");
            killed += 1;
        }
    }
    killed
}
