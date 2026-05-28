//! Terminal / CI environment detection for ANSI rendering decisions.
//!
//! The CLI's "should I emit color?" decision lives here so all colored output
//! paths (task lifecycle, bazelrc error rendering, etc.) agree on the answer.

use std::io::IsTerminal as _;

/// True when stderr is a TTY or we're on a recognized CI host, AND `NO_COLOR`
/// is either unset or empty.
///
/// Honors the [NO_COLOR](https://no-color.org/) convention: disable iff the
/// variable is present *and non-empty*. `NO_COLOR= my-cmd` is the standard
/// shell idiom for clearing it for a single invocation; treat empty as unset.
pub fn color_enabled() -> bool {
    if std::env::var("NO_COLOR").is_ok_and(|v| !v.is_empty()) {
        return false;
    }
    std::io::stderr().is_terminal() || on_recognized_ci()
}

/// True on any recognized CI host (Buildkite, GitHub Actions, CircleCI, GitLab
/// CI, or generic `CI=...`). Used to force-enable color when stderr is piped
/// to a CI log viewer that renders ANSI even without a TTY, and to gate other
/// CI-only UX (task-key suffix on the running-task header, etc.).
pub fn on_recognized_ci() -> bool {
    std::env::var_os("BUILDKITE").is_some()
        || std::env::var_os("CI").is_some()
        || std::env::var_os("GITHUB_ACTIONS").is_some()
        || std::env::var_os("CIRCLECI").is_some()
        || std::env::var_os("GITLAB_CI").is_some()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Mutex;

    /// `color_enabled` and `on_recognized_ci` read process-wide env vars; the
    /// lock serializes test cases against each other so cargo's parallel
    /// runner doesn't race on the global state.
    static ENV_LOCK: Mutex<()> = Mutex::new(());

    /// Snapshot the set of env vars this module reads, clear them so the
    /// test starts from a known baseline, then restore on drop. Wraps the
    /// `unsafe` env mutation in one place.
    struct EnvGuard {
        saved: Vec<(&'static str, Option<std::ffi::OsString>)>,
    }

    impl EnvGuard {
        const VARS: &'static [&'static str] = &[
            "NO_COLOR",
            "BUILDKITE",
            "CI",
            "GITHUB_ACTIONS",
            "CIRCLECI",
            "GITLAB_CI",
        ];

        fn fresh() -> Self {
            let saved = Self::VARS
                .iter()
                .map(|&k| (k, std::env::var_os(k)))
                .collect();
            // SAFETY: this test module's env mutations are serialized via ENV_LOCK.
            unsafe {
                for k in Self::VARS {
                    std::env::remove_var(k);
                }
            }
            Self { saved }
        }

        fn set(&self, key: &str, value: &str) {
            // SAFETY: serialized via ENV_LOCK.
            unsafe { std::env::set_var(key, value) }
        }
    }

    impl Drop for EnvGuard {
        fn drop(&mut self) {
            // SAFETY: serialized via ENV_LOCK.
            unsafe {
                for (k, v) in &self.saved {
                    match v {
                        Some(val) => std::env::set_var(k, val),
                        None => std::env::remove_var(k),
                    }
                }
            }
        }
    }

    #[test]
    fn no_color_non_empty_disables_color() {
        let _lock = ENV_LOCK.lock().unwrap();
        let env = EnvGuard::fresh();
        env.set("NO_COLOR", "1");
        // Force a CI signal so the underlying source would otherwise enable color.
        env.set("CI", "true");
        assert!(!color_enabled(), "NO_COLOR=1 must disable even on CI");
    }

    #[test]
    fn no_color_empty_treated_as_unset_per_spec() {
        let _lock = ENV_LOCK.lock().unwrap();
        let env = EnvGuard::fresh();
        env.set("NO_COLOR", "");
        env.set("CI", "true");
        // Empty NO_COLOR should NOT disable; CI=true forces color on (no TTY in tests).
        assert!(
            color_enabled(),
            "NO_COLOR='' (empty) must be treated as unset per no-color.org"
        );
    }

    #[test]
    fn no_ci_env_and_no_tty_means_no_color() {
        let _lock = ENV_LOCK.lock().unwrap();
        let _env = EnvGuard::fresh();
        // No NO_COLOR set, no CI env set, and `cargo test` doesn't give us a
        // TTY on stderr, so color is off.
        assert!(!color_enabled());
        assert!(!on_recognized_ci());
    }

    #[test]
    fn each_recognized_ci_var_triggers_detection() {
        let _lock = ENV_LOCK.lock().unwrap();
        for var in EnvGuard::VARS.iter().filter(|v| **v != "NO_COLOR") {
            let env = EnvGuard::fresh();
            env.set(var, "true");
            assert!(on_recognized_ci(), "{var}=true should trigger CI detection");
        }
    }
}
