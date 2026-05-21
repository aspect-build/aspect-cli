use std::time::Duration;
use std::time::Instant;

/// Send `sigints` rounds of SIGINT to each PID, sleeping `between` between
/// rounds, then wait up to `grace` for the processes to exit. Any PIDs
/// still alive after `grace` are SIGKILL'd. Returns the PIDs that needed
/// SIGKILL (empty if all exited gracefully on SIGINT).
///
/// Synchronous; the caller is responsible for running it on a thread that
/// can block (e.g. `tokio::task::spawn_blocking` from an async context).
///
/// Two callers share this:
///   - `cancel.rs::force_kill` — the AXL `ctx.bazel.cancel_invocation()`
///     path, where the 1st SIGINT has already been sent by the AXL builtin
///     and this issues rounds 2 and 3 of bazel's 3-SIGINT protocol.
///   - `aspect-cli/main.rs::run_shutdown_sequence` — the OS-initiated
///     SIGINT/SIGTERM handler, which issues the full 3-SIGINT burst itself.
///
/// (See https://bazel.build/run/cancellation for the protocol bazel
/// implements on the receiving end.)
pub(crate) fn escalate(
    pids: &[u32],
    sigints: usize,
    between: Duration,
    grace: Duration,
) -> Vec<u32> {
    if pids.is_empty() {
        return Vec::new();
    }
    for round in 0..sigints {
        if round > 0 && !between.is_zero() {
            std::thread::sleep(between);
        }
        for &pid in pids {
            if is_pid_running(pid) {
                tracing::warn!(
                    "escalate: sending SIGINT {}/{} to PID {pid}",
                    round + 1,
                    sigints
                );
                sigint(pid);
            }
        }
    }

    let start = Instant::now();
    let poll = Duration::from_millis(100);
    loop {
        let alive: Vec<u32> = pids
            .iter()
            .copied()
            .filter(|&p| is_pid_running(p))
            .collect();
        if alive.is_empty() {
            return Vec::new();
        }
        if start.elapsed() >= grace {
            for &pid in &alive {
                tracing::warn!(
                    "escalate: PID {pid} did not exit after {}ms grace — sending SIGKILL",
                    grace.as_millis()
                );
                sigkill(pid);
            }
            return alive;
        }
        std::thread::sleep(poll);
    }
}

/// Probes whether a process with the given PID exists using signal 0.
#[cfg(unix)]
pub(crate) fn is_pid_running(pid: u32) -> bool {
    use nix::sys::signal;
    use nix::unistd::Pid;

    signal::kill(Pid::from_raw(pid as i32), None).is_ok()
}

#[cfg(not(unix))]
pub(crate) fn is_pid_running(_pid: u32) -> bool {
    false
}

/// Sends SIGKILL to the given PID. Silently ignores failures.
#[cfg(unix)]
pub(crate) fn sigkill(pid: u32) {
    use nix::sys::signal::{self, Signal};
    use nix::unistd::Pid;

    tracing::warn!("Sending SIGKILL to PID {}", pid);
    let _ = signal::kill(Pid::from_raw(pid as i32), Signal::SIGKILL);
}

#[cfg(not(unix))]
pub(crate) fn sigkill(_pid: u32) {
    tracing::warn!("sigkill is not supported on this platform");
}

/// Sends SIGINT to the given PID. Returns true if the signal was sent successfully.
#[cfg(unix)]
pub(crate) fn sigint(pid: u32) -> bool {
    use nix::sys::signal::{self, Signal};
    use nix::unistd::Pid;

    signal::kill(Pid::from_raw(pid as i32), Signal::SIGINT).is_ok()
}

#[cfg(not(unix))]
pub(crate) fn sigint(_pid: u32) -> bool {
    tracing::warn!("sigint is not supported on this platform");
    false
}

#[cfg(not(unix))]
pub(crate) fn sigterm(_pid: u32) -> bool {
    tracing::warn!("sigterm is not supported on this platform");
    false
}
