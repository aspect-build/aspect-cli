//! Warming gate for Aspect Workflows runners.
//!
//! The launcher's download cache (`ASPECT_LAUNCHER_CACHE`) lives under the
//! runner's ephemeral storage, which the *warming* bootstrap stage restores
//! from a cloud-storage archive. Warming runs concurrently with the first jobs
//! the runner accepts and only finishes populating and taking ownership of the
//! cache directory once it is done. Using the cache before then races that
//! restore, so the launcher blocks until warming signals completion.
//!
//! The runner agent communicates warming state through environment variables it
//! sets on every job. This module reads them and is a no-op anywhere they are
//! absent (off-runner, or a runner with warming disabled), leaving local usage
//! unaffected.
//!
//! The behavior mirrors the CLI's own `_wait_for_warming` health-check step: a
//! 1s poll with no timeout, since the bootstrap terminates the instance if the
//! concurrent restore hits a critical error, so the loop cannot hang forever.

use std::env::var;
use std::path::Path;
use std::thread::sleep;
use std::time::Duration;

/// Set by the runner bootstrap only when warming is enabled for the runner's
/// queue. Absence means there is nothing to wait for.
const WARMING_ENABLED_ENV: &str = "ASPECT_WORKFLOWS_RUNNER_WARMING_ENABLED";

/// Path to the marker file the warming stage writes once the restore has
/// finished and the cache directory has been handed back to the runner user.
/// The path is set by the bootstrap; the file itself appears only on completion.
const WARMING_COMPLETE_MARKER_ENV: &str = "ASPECT_WORKFLOWS_RUNNER_WARMING_COMPLETE_MARKER_FILE";

/// Re-check interval for the completion marker, matching the CLI health check.
const POLL_INTERVAL: Duration = Duration::from_secs(1);

/// Whether warming is enabled for this runner.
fn warming_enabled() -> bool {
    var(WARMING_ENABLED_ENV).is_ok_and(|v| !v.is_empty())
}

/// The completion-marker path, or `None` when the bootstrap did not publish one.
///
/// Without a path, completion can never be observed, so callers must not wait.
fn marker_path() -> Option<String> {
    var(WARMING_COMPLETE_MARKER_ENV)
        .ok()
        .filter(|p| !p.is_empty())
}

/// Block until warming completes so the on-disk cache is fully restored and
/// owned by the runner user before the launcher touches it.
///
/// Returns immediately when warming is not enabled, when the completion marker
/// has already appeared, or — to avoid waiting on a signal that can never
/// arrive — when warming is enabled but no marker path was published.
pub fn wait_for_warming() {
    if !warming_enabled() {
        return;
    }

    let Some(marker) = marker_path() else {
        eprintln!(
            "warming is enabled but {WARMING_COMPLETE_MARKER_ENV} is not set; \
             not waiting for warming to complete"
        );
        return;
    };

    let marker_path = Path::new(&marker);
    if marker_path.exists() {
        return;
    }

    eprintln!("waiting for warming to complete before using the aspect cache...");
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

    /// `warming_enabled`/`marker_path` read process-global env vars, so tests
    /// that set them must run serially.
    fn env_guard() -> MutexGuard<'static, ()> {
        static LOCK: OnceLock<Mutex<()>> = OnceLock::new();
        LOCK.get_or_init(|| Mutex::new(()))
            .lock()
            .unwrap_or_else(|e| e.into_inner())
    }

    fn set_enabled(value: Option<&str>) {
        unsafe {
            match value {
                Some(v) => std::env::set_var(WARMING_ENABLED_ENV, v),
                None => std::env::remove_var(WARMING_ENABLED_ENV),
            }
        }
    }

    fn set_marker(value: Option<&str>) {
        unsafe {
            match value {
                Some(v) => std::env::set_var(WARMING_COMPLETE_MARKER_ENV, v),
                None => std::env::remove_var(WARMING_COMPLETE_MARKER_ENV),
            }
        }
    }

    fn clear_env() {
        set_enabled(None);
        set_marker(None);
    }

    /// A unique, non-existent path under the per-process temp dir.
    fn missing_marker(label: &str) -> std::path::PathBuf {
        let p = std::env::temp_dir().join(format!(
            "aspect-launcher-warming-test-{}-{}",
            std::process::id(),
            label
        ));
        let _ = std::fs::remove_file(&p);
        p
    }

    #[test]
    fn warming_enabled_is_true_only_for_a_nonempty_value() {
        let _g = env_guard();
        clear_env();
        assert!(!warming_enabled(), "unset should be disabled");
        set_enabled(Some(""));
        assert!(!warming_enabled(), "empty should be disabled");
        set_enabled(Some("1"));
        assert!(warming_enabled());
        clear_env();
    }

    #[test]
    fn marker_path_is_none_when_unset_or_empty() {
        let _g = env_guard();
        clear_env();
        assert_eq!(marker_path(), None, "unset");
        set_marker(Some(""));
        assert_eq!(marker_path(), None, "empty");
        set_marker(Some("/tmp/marker"));
        assert_eq!(marker_path(), Some("/tmp/marker".to_string()));
        clear_env();
    }

    /// All three early-return paths must return without blocking. The marker is
    /// pointed at a path that never appears, so a wrongful wait would hang.
    #[test]
    fn returns_immediately_without_blocking() {
        let _g = env_guard();

        // Warming disabled.
        clear_env();
        set_marker(Some(missing_marker("disabled").to_str().unwrap()));
        wait_for_warming();

        // Enabled with no marker path published.
        clear_env();
        set_enabled(Some("1"));
        wait_for_warming();

        // Marker already present (the temp dir itself exists).
        clear_env();
        set_enabled(Some("1"));
        set_marker(Some(std::env::temp_dir().to_str().unwrap()));
        wait_for_warming();

        clear_env();
    }

    #[test]
    fn waits_until_marker_appears() {
        let _g = env_guard();
        clear_env();

        let marker = missing_marker("appears");
        set_enabled(Some("1"));
        set_marker(Some(marker.to_str().unwrap()));

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

        assert!(marker.exists(), "must only return once the marker exists");
        assert!(
            waited >= POLL_INTERVAL,
            "expected to block until the marker appeared, waited {waited:?}"
        );

        let _ = std::fs::remove_file(&marker);
        clear_env();
    }
}
