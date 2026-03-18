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
