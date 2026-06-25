//! Cold-start warming gate.
//!
//! On Aspect Workflows CI runners the launcher's download cache
//! (`ASPECT_LAUNCHER_CACHE`, i.e. `${storage}/caches/aspect-launcher`) lives
//! under the ephemeral mount that the *warming* bootstrap stage restores from a
//! cloud-storage archive. Warming runs asynchronously and the runner agent
//! begins accepting jobs before it finishes (the bootstrap only blocks on
//! warming at the very end, after the agent is started). During the restore the
//! cache directory is `rm -rf`'d and re-extracted via `sudo tar` owned by
//! `root`; ownership is handed back to the `aspect-runner` user only once the
//! whole restore completes. The completion marker file is written *after* that
//! final chown.
//!
//! So if the launcher touches the cache during that window it hits
//! `Permission denied (os error 13)` — the directory is transiently root-owned
//! while the job runs as `aspect-runner`. To avoid the race entirely we treat
//! the warming-complete marker as the signal that the cache directory is
//! settled and owned by `aspect-runner`, and block until it appears before
//! provisioning the CLI.
//!
//! This is a no-op anywhere the warming env vars are absent (any non-Workflows
//! machine, or a runner with warming disabled), so it has zero effect on local
//! or non-warmed usage.

use std::env::var;
use std::path::Path;
use std::thread::sleep;
use std::time::Duration;

/// Set to `1` by the runner bootstrap only when warming is enabled for the
/// runner's queue. Absent means warming is disabled (or we are not on a
/// Workflows runner) and there is nothing to wait for.
const WARMING_ENABLED_ENV: &str = "ASPECT_WORKFLOWS_RUNNER_WARMING_ENABLED";

/// Path to the marker file the warming stage writes once the restore has
/// finished and the cache directories have been chowned back to the runner
/// user. Always set by the bootstrap; the file itself appears only on
/// completion.
const WARMING_COMPLETE_MARKER_ENV: &str = "ASPECT_WORKFLOWS_RUNNER_WARMING_COMPLETE_MARKER_FILE";

/// How often to re-check for the completion marker. Matches the 1s poll in the
/// runner's `agent_health_check.sh` `wait_for_warming`.
const POLL_INTERVAL: Duration = Duration::from_secs(1);

/// If warming is enabled but has not yet completed, block until the completion
/// marker appears so the on-disk cache is settled (and runner-owned) before the
/// launcher touches it.
///
/// There is intentionally no timeout: a warming restore can legitimately take
/// minutes, and the surrounding CI job has its own timeout that will terminate
/// the build if warming is genuinely stuck. We mirror the health-check step's
/// indefinite poll rather than racing the cache.
///
/// No-ops (returns immediately) when:
/// - warming is not enabled (`WARMING_ENABLED_ENV` unset/empty), or
/// - the marker path is unset/empty (misconfiguration — we do not wedge on it), or
/// - the marker already exists (warming already finished).
pub fn wait_for_warming() {
    if var(WARMING_ENABLED_ENV).map(|v| v != "1").unwrap_or(true) {
        return;
    }

    let marker = match var(WARMING_COMPLETE_MARKER_ENV) {
        Ok(p) if !p.is_empty() => p,
        _ => {
            // Warming is enabled but we were not told where the marker is. Don't
            // block forever on a misconfiguration; proceed and let the cache
            // operations succeed or report their own error.
            eprintln!(
                "warming is enabled but {} is not set; not waiting for warming",
                WARMING_COMPLETE_MARKER_ENV
            );
            return;
        }
    };

    let marker_path = Path::new(&marker);
    if marker_path.exists() {
        return;
    }

    eprintln!(
        "waiting for warming to complete before using the aspect cache (marker: {})...",
        marker
    );
    while !marker_path.exists() {
        sleep(POLL_INTERVAL);
    }
    eprintln!("warming complete, continuing");
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::{Mutex, MutexGuard, OnceLock};
    use std::time::Instant;

    // `wait_for_warming` reads process-global env vars, so the tests that set
    // them must not run concurrently. Serialize them on a shared lock.
    fn env_guard() -> MutexGuard<'static, ()> {
        static LOCK: OnceLock<Mutex<()>> = OnceLock::new();
        LOCK.get_or_init(|| Mutex::new(()))
            .lock()
            .unwrap_or_else(|e| e.into_inner())
    }

    fn clear_env() {
        unsafe {
            std::env::remove_var(WARMING_ENABLED_ENV);
            std::env::remove_var(WARMING_COMPLETE_MARKER_ENV);
        }
    }

    /// A path under the per-process temp dir that does not exist, unique per
    /// label so parallel-built test binaries don't collide.
    fn missing_marker(label: &str) -> std::path::PathBuf {
        std::env::temp_dir().join(format!(
            "aspect-launcher-warming-test-{}-{}",
            std::process::id(),
            label
        ))
    }

    #[test]
    fn no_op_when_warming_disabled() {
        let _g = env_guard();
        clear_env();
        // Marker points at a path that will never exist; if we wrongly waited,
        // the test would hang. WARMING_ENABLED is unset, so it must return.
        unsafe {
            std::env::set_var(
                WARMING_COMPLETE_MARKER_ENV,
                missing_marker("disabled").to_str().unwrap(),
            );
        }
        wait_for_warming();
        clear_env();
    }

    #[test]
    fn no_op_when_marker_already_present() {
        let _g = env_guard();
        clear_env();
        // Any path that exists works as a present marker; the temp dir itself does.
        let existing = std::env::temp_dir();
        unsafe {
            std::env::set_var(WARMING_ENABLED_ENV, "1");
            std::env::set_var(WARMING_COMPLETE_MARKER_ENV, existing.to_str().unwrap());
        }
        wait_for_warming();
        clear_env();
    }

    #[test]
    fn no_op_when_enabled_but_marker_path_empty() {
        let _g = env_guard();
        clear_env();
        // Enabled but no marker path => must not block (misconfiguration guard).
        unsafe {
            std::env::set_var(WARMING_ENABLED_ENV, "1");
            std::env::set_var(WARMING_COMPLETE_MARKER_ENV, "");
        }
        wait_for_warming();
        clear_env();
    }

    #[test]
    fn no_op_when_enabled_value_is_not_one() {
        let _g = env_guard();
        clear_env();
        // Only the literal "1" enables the wait.
        unsafe {
            std::env::set_var(WARMING_ENABLED_ENV, "true");
            std::env::set_var(
                WARMING_COMPLETE_MARKER_ENV,
                missing_marker("not-one").to_str().unwrap(),
            );
        }
        wait_for_warming();
        clear_env();
    }

    #[test]
    fn waits_until_marker_appears() {
        let _g = env_guard();
        clear_env();

        let marker = missing_marker("appears");
        let _ = std::fs::remove_file(&marker);
        assert!(!marker.exists());

        unsafe {
            std::env::set_var(WARMING_ENABLED_ENV, "1");
            std::env::set_var(WARMING_COMPLETE_MARKER_ENV, marker.to_str().unwrap());
        }

        // Create the marker shortly after the wait begins, from another thread.
        let marker_for_thread = marker.clone();
        let writer = std::thread::spawn(move || {
            std::thread::sleep(Duration::from_millis(1500));
            std::fs::write(&marker_for_thread, b"done").unwrap();
        });

        let start = Instant::now();
        wait_for_warming();
        let waited = start.elapsed();

        writer.join().unwrap();

        // It must have actually blocked (the 1s poll means it returns ~2s after
        // the 1.5s write), and only returned once the marker existed.
        assert!(marker.exists());
        assert!(
            waited >= Duration::from_secs(1),
            "expected to block until the marker appeared, waited {:?}",
            waited
        );

        let _ = std::fs::remove_file(&marker);
        clear_env();
    }
}
