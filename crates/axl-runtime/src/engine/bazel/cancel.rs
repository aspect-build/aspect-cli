use allocative::Allocative;
use derive_more::Display;
use starlark::environment::Methods;
use starlark::environment::MethodsBuilder;
use starlark::environment::MethodsStatic;
use starlark::starlark_module;
use starlark::values;
use starlark::values::AllocValue;
use starlark::values::Heap;
use starlark::values::NoSerialize;
use starlark::values::ProvidesStaticType;
use starlark::values::Trace;
use starlark::values::ValueLike;
use starlark::values::starlark_value;

use super::info;
use super::process;

#[derive(Debug, ProvidesStaticType, Display, Trace, NoSerialize, Allocative)]
#[display("<bazel.build.Cancellation>")]
pub struct Cancellation {
    #[allocative(skip)]
    startup_flags: Vec<String>,
    #[allocative(skip)]
    force_kill_after_ms: u64,
}

impl Cancellation {
    pub fn new(startup_flags: Vec<String>, force_kill_after_ms: u64) -> Self {
        Self {
            startup_flags,
            force_kill_after_ms,
        }
    }
}

impl<'v> AllocValue<'v> for Cancellation {
    fn alloc_value(self, heap: Heap<'v>) -> values::Value<'v> {
        heap.alloc_simple(self)
    }
}

#[starlark_value(type = "bazel.build.Cancellation")]
impl<'v> values::StarlarkValue<'v> for Cancellation {
    fn get_methods() -> Option<&'static Methods> {
        static RES: MethodsStatic = MethodsStatic::new();
        RES.methods(cancellation_methods)
    }
}

#[starlark_module]
pub(crate) fn cancellation_methods(registry: &mut MethodsBuilder) {
    /// Whether the bazel server is currently busy (lock held by another client).
    /// Queries in real time via `bazel --noblock_for_lock info`.
    #[starlark(attribute)]
    fn busy<'v>(this: values::Value<'v>) -> anyhow::Result<bool> {
        let cancellation = this.downcast_ref::<Cancellation>().unwrap();
        Ok(info::is_server_busy(&cancellation.startup_flags))
    }

    /// Block until the cancelled invocation finishes.
    ///
    /// Polls until the server is no longer busy. If the `force_kill_after_ms`
    /// deadline (set on `cancel_invocation`) is reached while still busy,
    /// automatically escalates by calling `force()`.
    ///
    /// Returns `True` if the server became free (either gracefully or after
    /// force-kill). Returns `False` only if `timeout_ms` is set and reached
    /// before the server became free (in this case no automatic escalation
    /// occurs — use `force()` manually).
    fn wait<'v>(
        this: values::Value<'v>,
        #[starlark(require = named, default = 200)] poll_ms: i32,
        #[starlark(require = named, default = 0)] timeout_ms: i32,
    ) -> anyhow::Result<bool> {
        let cancellation = this.downcast_ref::<Cancellation>().unwrap();
        let poll_ms = poll_ms.max(0) as u64;
        let timeout_ms = timeout_ms.max(0) as u64;
        let force_kill_after_ms = cancellation.force_kill_after_ms;

        validate_wait_params(timeout_ms, force_kill_after_ms)?;

        let start = std::time::Instant::now();

        while info::is_server_busy(&cancellation.startup_flags) {
            let elapsed = start.elapsed();

            // Manual timeout: return False without escalation.
            if timeout_ms > 0 && elapsed >= std::time::Duration::from_millis(timeout_ms) {
                return Ok(false);
            }

            // Auto-escalation deadline: force-kill and continue waiting.
            if force_kill_after_ms > 0
                && elapsed >= std::time::Duration::from_millis(force_kill_after_ms)
            {
                force_kill(&cancellation.startup_flags);
                // After force-kill, wait indefinitely for the server to stop.
                // Reset by breaking out and falling through to return true.
                while info::is_server_busy(&cancellation.startup_flags) {
                    std::thread::sleep(std::time::Duration::from_millis(poll_ms));
                }
                return Ok(true);
            }

            std::thread::sleep(std::time::Duration::from_millis(poll_ms));
        }
        Ok(true)
    }

    /// Forcefully cancel the invocation.
    ///
    /// Sends the 2nd and 3rd SIGINT to the Bazel client, following Bazel's
    /// 3-stage cancellation protocol (the 1st SIGINT was already sent by
    /// `cancel_invocation()`). The 3rd SIGINT triggers Bazel's built-in
    /// `KillServerProcess` which kills the server and exits the client.
    ///
    /// If the client doesn't exit within 5 seconds, falls back to SIGKILL
    /// on both the client and server. If no client is found holding the lock
    /// (e.g. the client crashed), sends SIGKILL directly to the server daemon.
    ///
    /// Returns `True` if a signal was sent, `False` if neither the client nor
    /// server could be found (the build may have already finished).
    fn force<'v>(this: values::Value<'v>) -> anyhow::Result<bool> {
        let cancellation = this.downcast_ref::<Cancellation>().unwrap();
        Ok(force_kill(&cancellation.startup_flags))
    }
}

/// Validate that timeout_ms and force_kill_after_ms are not used in
/// conflicting ways that would cause wait() to hang or behave ambiguously.
fn validate_wait_params(timeout_ms: u64, force_kill_after_ms: u64) -> anyhow::Result<()> {
    if timeout_ms > 0 && force_kill_after_ms > 0 {
        return Err(anyhow::anyhow!(
            "wait(timeout_ms) cannot be used with cancel_invocation(force_kill_after_ms). \
             Use force_kill_after_ms = 0 to disable auto-escalation and manage \
             cancellation manually with wait(timeout_ms) and force()."
        ));
    }

    if timeout_ms == 0 && force_kill_after_ms == 0 {
        return Err(anyhow::anyhow!(
            "wait() with no timeout requires force_kill_after_ms to be set on \
             cancel_invocation(). Either use the default force_kill_after_ms or \
             pass timeout_ms to wait() and handle escalation manually with force()."
        ));
    }

    Ok(())
}

/// How long to wait for the Bazel client to exit after a second SIGINT
/// before escalating to SIGKILL.
const FORCE_KILL_TIMEOUT_MS: u64 = 5000;
const FORCE_KILL_POLL_MS: u64 = 100;

/// Send a forceful cancellation signal following Bazel's 3-SIGINT protocol.
///
/// cancel_invocation() already sent the 1st SIGINT (graceful cancel). This
/// function sends the 2nd and 3rd SIGINTs to escalate:
///   - 2nd SIGINT: repeated CancelRequest (Bazel is already cancelling)
///   - 3rd SIGINT: triggers Bazel's built-in KillServerProcess + client exit
///
/// If the client still doesn't exit after the 3rd SIGINT, we SIGKILL both
/// the client and server ourselves as a last resort.
fn force_kill(startup_flags: &[String]) -> bool {
    if let Some(client_pid) = info::client_pid(startup_flags) {
        // 2nd SIGINT: repeated cancel request.
        tracing::warn!("cancel_invocation: sending 2nd SIGINT to Bazel client PID {client_pid}");
        process::sigint(client_pid);

        // 3rd SIGINT: triggers Bazel's built-in server kill + client exit.
        tracing::warn!("cancel_invocation: sending 3rd SIGINT to Bazel client PID {client_pid}");
        process::sigint(client_pid);

        // Monitor the client — if it doesn't exit within the timeout,
        // SIGKILL both the client and the server ourselves.
        let start = std::time::Instant::now();
        while process::is_pid_running(client_pid) {
            if start.elapsed() >= std::time::Duration::from_millis(FORCE_KILL_TIMEOUT_MS) {
                tracing::warn!(
                    "cancel_invocation: Bazel client PID {client_pid} did not exit \
                     after {FORCE_KILL_TIMEOUT_MS}ms, sending SIGKILL"
                );
                process::sigkill(client_pid);
                if let Some(server_pid) = info::server_pid_nonblocking(startup_flags) {
                    tracing::warn!(
                        "cancel_invocation: also sending SIGKILL to Bazel server PID \
                         {server_pid}"
                    );
                    process::sigkill(server_pid);
                }
                return true;
            }
            std::thread::sleep(std::time::Duration::from_millis(FORCE_KILL_POLL_MS));
        }
        return true;
    }

    // Client is gone (crashed or already exited). Only SIGKILL the server
    // if it's still busy — otherwise there's nothing to cancel.
    if info::is_server_busy(startup_flags) {
        if let Some(pid) = info::server_pid_nonblocking(startup_flags) {
            tracing::warn!(
                "cancel_invocation: Bazel client not found, sending SIGKILL to \
                 server PID {pid}"
            );
            process::sigkill(pid);
            return true;
        }
    }
    false
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn validate_auto_escalation_with_default_params() {
        // force_kill_after_ms=5000 (default), timeout_ms=0 (default) → OK
        assert!(validate_wait_params(0, 5000).is_ok());
    }

    #[test]
    fn validate_manual_timeout() {
        // force_kill_after_ms=0, timeout_ms=5000 → OK
        assert!(validate_wait_params(5000, 0).is_ok());
    }

    #[test]
    fn validate_both_set_is_error() {
        // force_kill_after_ms=5000, timeout_ms=1000 → error
        let err = validate_wait_params(1000, 5000).unwrap_err();
        assert!(
            err.to_string().contains("cannot be used with"),
            "unexpected error: {err}"
        );
    }

    #[test]
    fn validate_neither_set_is_error() {
        // force_kill_after_ms=0, timeout_ms=0 → error (would hang forever)
        let err = validate_wait_params(0, 0).unwrap_err();
        assert!(
            err.to_string().contains("no timeout"),
            "unexpected error: {err}"
        );
    }
}
