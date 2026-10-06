//! Process-wide registry of process groups started by `std.process.Command`.
//!
//! A child started in its own process group does not receive the terminal's
//! Ctrl-C, which only reaches the foreground group. The binary's signal handler
//! interrupts every group registered here so those children can shut down
//! instead of outliving aspect-cli.

use std::sync::Mutex;
use std::sync::OnceLock;

fn registry() -> &'static Mutex<Vec<i32>> {
    static REG: OnceLock<Mutex<Vec<i32>>> = OnceLock::new();
    REG.get_or_init(|| Mutex::new(Vec::new()))
}

/// Register a process group as live until the returned guard is dropped.
#[must_use = "the group is unregistered when the guard is dropped"]
pub fn register(pgid: i32) -> LiveGroupGuard {
    if let Ok(mut g) = registry().lock() {
        g.push(pgid);
    }
    LiveGroupGuard { pgid }
}

/// Snapshot of registered process groups.
pub fn live_groups() -> Vec<i32> {
    registry().lock().map(|g| g.clone()).unwrap_or_default()
}

/// Send SIGINT to every registered process group.
pub fn interrupt_all() {
    for pgid in live_groups() {
        tracing::warn!("received OS shutdown signal — sending SIGINT to process group {pgid}");
        let _ = interrupt(pgid);
    }
}

/// Whether any registered process group still has a member.
pub fn any_running() -> bool {
    live_groups().into_iter().any(is_running)
}

/// SIGKILL every registered process group that still has a member. Returns
/// the number of groups killed.
pub fn force_kill_all_remaining() -> usize {
    let mut killed = 0;
    for pgid in live_groups() {
        if is_running(pgid) {
            tracing::warn!(
                "process group {pgid} did not exit after SIGINT grace — sending SIGKILL"
            );
            let _ = kill(pgid);
            killed += 1;
        }
    }
    killed
}

#[cfg(unix)]
fn signal(pgid: i32, signal: Option<nix::sys::signal::Signal>) -> nix::Result<()> {
    nix::sys::signal::kill(nix::unistd::Pid::from_raw(-pgid), signal)
}

/// Send SIGINT to `pgid`. A group with no members is not an error.
#[cfg(unix)]
pub(crate) fn interrupt(pgid: i32) -> nix::Result<()> {
    match signal(pgid, Some(nix::sys::signal::Signal::SIGINT)) {
        Err(nix::errno::Errno::ESRCH) => Ok(()),
        result => result,
    }
}

/// Send SIGKILL to `pgid`. A group with no members is not an error.
#[cfg(unix)]
pub(crate) fn kill(pgid: i32) -> nix::Result<()> {
    match signal(pgid, Some(nix::sys::signal::Signal::SIGKILL)) {
        Err(nix::errno::Errno::ESRCH) => Ok(()),
        result => result,
    }
}

/// Whether `pgid` still has a member, counting unreaped zombies.
#[cfg(unix)]
pub(crate) fn is_running(pgid: i32) -> bool {
    signal(pgid, None).is_ok()
}

#[cfg(not(unix))]
fn interrupt(_pgid: i32) -> std::io::Result<()> {
    Ok(())
}

#[cfg(not(unix))]
fn kill(_pgid: i32) -> std::io::Result<()> {
    Ok(())
}

#[cfg(not(unix))]
fn is_running(_pgid: i32) -> bool {
    false
}

/// Removes its process group from the registry on drop.
#[derive(Debug)]
pub struct LiveGroupGuard {
    pgid: i32,
}

impl Drop for LiveGroupGuard {
    fn drop(&mut self) {
        if let Ok(mut g) = registry().lock() {
            if let Some(idx) = g.iter().position(|p| *p == self.pgid) {
                g.swap_remove(idx);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn registers_and_drops() {
        let guard = register(-111);
        assert!(live_groups().contains(&-111));
        drop(guard);
        assert!(!live_groups().contains(&-111));
    }
}
