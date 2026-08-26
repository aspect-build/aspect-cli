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
    own_group: bool,
) -> std::io::Result<(process::Child, LiveChildGuard)> {
    supervisor().spawn(command, own_group)
}

/// Prevents new children from spawning during process shutdown.
pub fn begin_shutdown() {
    supervisor().begin_shutdown();
}

/// Returns a snapshot of registered child PIDs.
pub fn live_pids() -> Vec<u32> {
    supervisor().live_pids()
}

/// Sends SIGTERM to registered children. A child in its own process group
/// never saw the terminal's SIGINT, so it is always signaled — as its whole
/// group; `skip_tty_signaled` skips only the children sharing our group,
/// which the tty already interrupted.
pub fn sigterm_all_for_shutdown(skip_tty_signaled: bool) {
    for (pid, own_group) in supervisor().entries() {
        let sent = if own_group {
            crate::engine::process::sigterm_group(pid).is_ok()
        } else if skip_tty_signaled {
            false
        } else {
            crate::engine::process::sigterm(pid)
        };
        if sent {
            tracing::warn!("sent SIGTERM to live child PID {pid}");
        }
    }
}

/// SIGKILLs registered children that are still running and returns the count.
pub fn force_kill_all_remaining() -> usize {
    let mut killed = 0;
    for (pid, own_group) in supervisor().entries() {
        let sent = if own_group {
            crate::engine::process::sigkill_group(pid).is_ok()
        } else {
            crate::engine::process::sigkill(pid)
        };
        if sent {
            tracing::warn!("sent SIGKILL to child PID {pid} after SIGTERM grace");
            killed += 1;
        }
    }
    killed
}
