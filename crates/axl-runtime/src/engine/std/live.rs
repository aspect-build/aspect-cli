//! Process-wide registry of live AXL-spawned child processes used to forward
//! shutdown signals.

use std::process;
use std::sync::OnceLock;

use crate::engine::process::{ProcessGuard, ProcessSupervisor};

fn supervisor() -> &'static ProcessSupervisor {
    static SUPERVISOR: OnceLock<ProcessSupervisor> = OnceLock::new();
    SUPERVISOR.get_or_init(ProcessSupervisor::default)
}

pub type LiveChildGuard = ProcessGuard;

pub(crate) fn spawn_registered(
    command: &mut process::Command,
) -> std::io::Result<(process::Child, LiveChildGuard)> {
    supervisor().spawn(command)
}

/// Prevents new children from spawning during process shutdown.
pub fn begin_shutdown() {
    supervisor().begin_shutdown();
}

/// Returns a snapshot of registered child PIDs.
pub fn live_pids() -> Vec<u32> {
    supervisor().live_pids()
}

/// Sends SIGTERM to every registered child that is still running.
pub fn sigterm_all_for_shutdown() {
    for pid in live_pids() {
        if crate::engine::process::sigterm(pid) {
            tracing::warn!("sent SIGTERM to live child PID {pid}");
        }
    }
}

/// SIGKILLs registered children that are still running and returns the count.
pub fn force_kill_all_remaining() -> usize {
    let mut killed = 0;
    for pid in live_pids() {
        if crate::engine::process::sigkill(pid) {
            tracing::warn!("sent SIGKILL to child PID {pid} after SIGTERM grace");
            killed += 1;
        }
    }
    killed
}
