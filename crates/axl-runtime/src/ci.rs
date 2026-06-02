//! Detection of recognized CI/CD hosts.
//!
//! A single source of truth so the several places that change behavior on CI
//! (cancellation strategy, task-header formatting, color forcing) agree on
//! what "on CI" means.

/// Environment variables whose presence marks a recognized CI host.
///
/// `CI` is the universal marker — every recognized host sets it — and the
/// named vars cover hosts that historically didn't always set it.
const CI_MARKERS: &[&str] = &["CI", "BUILDKITE", "GITHUB_ACTIONS", "CIRCLECI", "GITLAB_CI"];

/// True on any recognized CI host (Buildkite / GitHub Actions / CircleCI /
/// GitLab CI / generic `CI=…`).
///
/// Presence, not truthiness, is what counts: hosts sometimes export `CI=`
/// (empty), and that still marks a CI host.
pub fn on_recognized_ci() -> bool {
    is_ci_from(|var| std::env::var_os(var).is_some())
}

/// Pure core of [`on_recognized_ci`], parameterized over the env lookup so it
/// can be tested without mutating the process-global environment.
fn is_ci_from(present: impl Fn(&str) -> bool) -> bool {
    CI_MARKERS.iter().any(|var| present(var))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn false_when_no_markers_present() {
        assert!(!is_ci_from(|_| false));
    }

    #[test]
    fn true_for_each_recognized_marker() {
        for marker in CI_MARKERS {
            assert!(
                is_ci_from(|var| var == *marker),
                "expected CI detection when only {marker} is present"
            );
        }
    }

    #[test]
    fn unrecognized_var_does_not_count() {
        assert!(!is_ci_from(|var| var == "NOT_A_CI_MARKER"));
    }
}
