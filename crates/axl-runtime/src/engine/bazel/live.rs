//! Process-wide registry of live bazel client subprocesses.
//!
//! Every bazel `Command::spawn()` in this module registers the spawned
//! child's PID via [`register`]. The returned [`LiveBazelGuard`]
//! auto-unregisters the PID when dropped (typically when the build
//! completes and the `Child` is dropped).
//!
//! On OS signal (SIGINT / SIGTERM to aspect-cli), the binary's signal
//! handler iterates [`live_pids`] and sends SIGINT to each registered
//! client so the bazel subprocesses don't outlive aspect-cli. Without
//! this, a CI cancellation can leave bazel clients orphaned — they
//! hold the JVM-server lock and block every subsequent invocation on
//! that warm runner.

use std::sync::Mutex;
use std::sync::OnceLock;

use super::process;

fn registry() -> &'static Mutex<Vec<u32>> {
    static REG: OnceLock<Mutex<Vec<u32>>> = OnceLock::new();
    REG.get_or_init(|| Mutex::new(Vec::new()))
}

/// Register a bazel client PID as live. The returned guard removes
/// the PID from the registry when dropped — happens automatically
/// when the `Child` handle owning the spawn falls out of scope.
#[must_use = "drop the guard at the end of the bazel invocation; \
              if you never bind it, the registry won't track the PID"]
pub fn register(pid: u32) -> LiveBazelGuard {
    if let Ok(mut g) = registry().lock() {
        g.push(pid);
    }
    LiveBazelGuard { pid }
}

/// Spawn a `Command` and immediately register the resulting child's
/// PID. Returns `(Child, LiveBazelGuard)`. Bind the guard to a name
/// (typically `_guard`) for the lifetime of the subprocess —
/// usually until `child.wait()` / `child.wait_with_output()`
/// returns. On drop the PID is unregistered.
///
/// Wrapper around the standard `cmd.spawn()` flow that a CI-cancel
/// signal handler can reach. Without registration, a hung bazel
/// invocation won't receive the SIGINT/SIGKILL escalation when
/// aspect-cli is told to shut down.
pub fn spawn_registered(
    cmd: &mut std::process::Command,
) -> std::io::Result<(std::process::Child, LiveBazelGuard)> {
    let child = cmd.spawn()?;
    let guard = register(child.id());
    Ok((child, guard))
}

/// Snapshot of currently-live bazel client PIDs. Used by the OS
/// signal handler in `aspect-cli/src/main.rs` to forward cancellation.
pub fn live_pids() -> Vec<u32> {
    registry().lock().map(|g| g.clone()).unwrap_or_default()
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
        if process::is_pid_running(pid) {
            tracing::warn!(
                "received OS shutdown signal — sending SIGINT to live bazel client PID {pid}"
            );
            process::sigint(pid);
        }
    }
}

/// Best-effort SIGKILL to every registered bazel client that's still
/// alive. Used as the post-grace escalation when SIGINT didn't get
/// the client to exit. Returns the number of clients SIGKILL'd.
pub fn force_kill_all_remaining() -> usize {
    let mut killed = 0;
    for pid in live_pids() {
        if process::is_pid_running(pid) {
            tracing::warn!(
                "live bazel client PID {pid} did not exit after SIGINT grace — sending SIGKILL"
            );
            process::sigkill(pid);
            killed += 1;
        }
    }
    killed
}

/// RAII guard returned by [`register`]. Removes the PID from the
/// registry on drop. Multiple registrations of the same PID are fine
/// — drop removes the first matching entry.
#[derive(Debug)]
pub struct LiveBazelGuard {
    pid: u32,
}

impl Drop for LiveBazelGuard {
    fn drop(&mut self) {
        if let Ok(mut g) = registry().lock() {
            if let Some(idx) = g.iter().position(|p| *p == self.pid) {
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
        let _g1 = register(111);
        let _g2 = register(222);
        let live = live_pids();
        assert!(live.contains(&111));
        assert!(live.contains(&222));
        drop(_g1);
        assert!(!live_pids().contains(&111));
        assert!(live_pids().contains(&222));
        drop(_g2);
        assert!(!live_pids().contains(&222));
    }
}
