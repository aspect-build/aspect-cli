//! Process-wide registry of live bazel client subprocesses.
//!
//! Every bazel `Command::spawn()` in this module registers the spawned
//! child's PID via [`register`]. The returned [`LiveBazelGuard`]
//! auto-unregisters the PID when dropped (typically when the build
//! completes and the `Child` is dropped).
//!
//! On OS signal (SIGINT / SIGTERM to aspect-cli), the binary's signal
//! handler calls [`escalate_shutdown`] to run bazel's 3-SIGINT cancel
//! protocol against the registered set so the bazel subprocesses don't
//! outlive aspect-cli.
//!
//! Without this, a CI cancellation can hit bazel at a moment it can't
//! gracefully recover from. Two known flakes — both rare per
//! invocation, but bad when they fire on a warm runner:
//!   1. *Potential sandbox-state corruption* (bazelbuild/bazel#23880):
//!      a SIGKILL during sandbox cleanup can strand `_moved_trash_dir`
//!      in the sandbox base, and every subsequent invocation on that
//!      runner crashes in `afterCommand`. The health check
//!      ([`super::health_check`]) now detects and removes the directory
//!      on the next run so the runner recovers automatically.
//!   2. *Potential orphaned bazel client* holding the JVM-server lock:
//!      the next invocation on that runner hangs at "Running Bazel
//!      server needs to be killed" until the orphan exits.

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

/// Run bazel's 3-SIGINT cancel protocol against every registered bazel
/// client and SIGKILL any that don't exit within `grace`. Sync — the
/// caller must invoke it from a thread that can block (typically
/// `tokio::task::spawn_blocking` from the OS-signal handler).
///
/// Returns the number of clients SIGKILL'd (i.e. those that didn't
/// respond to `sigints` × SIGINT within `grace`).
///
/// The bazel protocol (see [`bazel cancellation docs`][1]):
///   1st SIGINT → graceful cancel of the in-flight command
///   2nd SIGINT → still graceful; gives a short window for cleanup
///   3rd SIGINT → triggers bazel's `KillServerProcess` and hard exit
///
/// Shares its escalation mechanics with the AXL-initiated
/// `cancel_invocation` path in [`super::cancel`] so the two
/// cancellation entry points stay in lock-step.
///
/// [1]: https://bazel.build/run/cancellation
pub fn escalate_shutdown(
    sigints: usize,
    between: std::time::Duration,
    grace: std::time::Duration,
) -> usize {
    process::escalate(&live_pids(), sigints, between, grace).len()
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

    /// `Build` stores the guard inside `RefCell<Option<…>>` so it can
    /// release the registration the moment the child is observed exited
    /// (rather than waiting for the Starlark object to be GC'd). Verify
    /// that `.take()` on the wrapped guard does in fact unregister the
    /// PID — this is the pattern `build.rs::wait()` / `try_wait()` rely
    /// on to keep us from SIGINT/SIGKILLing a reused PID.
    #[test]
    fn take_on_option_wrapped_guard_unregisters_pid() {
        use std::cell::RefCell;
        let cell = RefCell::new(Some(register(333)));
        assert!(live_pids().contains(&333));
        let _ = cell.borrow_mut().take();
        assert!(!live_pids().contains(&333));
    }
}
