//! Detection of recognized CI/CD hosts.
//!
//! A single source of truth so the several places that change behavior on CI
//! (cancellation strategy, task-header formatting, color forcing, auto task
//! naming) agree on what "on CI" means.

use std::path::PathBuf;

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

// ── Auto task names from the CI job ──────────────────────────────────────────
//
// When `--task:name` is unset, the CLI derives one. On a recognized CI host the
// name is `<kind>-<ci-job>` (e.g. `test-ci-linux`) so status checks read
// meaningfully and stay stable across runs. A `-2` / `-3` suffix disambiguates
// the same `<kind>-<job>` generated again on the same run *on the same machine*
// (e.g. one job script invoking the same kind twice). Off CI — or when no job
// name is exposed — the caller falls back to a random suffix.
//
// Matrix caveat: GitHub Actions exposes no per-shard env var (matrix values live
// only in `${{ matrix.* }}`; `GITHUB_JOB` is the shared YAML id), and shards run
// on separate machines, so every shard produces the same `<kind>-<job>`. Users
// who need per-shard status rows set an explicit `--task:name=<kind>-${{ matrix.x }}`.

/// Per-host env var carrying the CI job name, in detection-precedence order.
/// Each entry is `(host_marker, &[job_name_vars])`; the first present host whose
/// first non-empty var wins.
const CI_JOB_NAME_VARS: &[(&str, &[&str])] = &[
    ("GITHUB_ACTIONS", &["GITHUB_JOB"]),
    ("BUILDKITE", &["BUILDKITE_STEP_KEY", "BUILDKITE_LABEL"]),
    ("CIRCLECI", &["CIRCLE_JOB"]),
    ("GITLAB_CI", &["CI_JOB_NAME"]),
];

/// Per-host 0-based shard index for steps using native parallelism. Each shard
/// shares the job name above but runs on a separate agent, so the local de-dup
/// claim file can't disambiguate them — the index must go into the name itself.
/// Set only when the step opted into parallelism; absent otherwise.
/// (GitHub Actions has no equivalent: matrix values aren't exported to the env.)
const CI_SHARD_INDEX_VARS: &[(&str, &[&str])] = &[
    ("BUILDKITE", &["BUILDKITE_PARALLEL_JOB"]),
    ("CIRCLECI", &["CIRCLE_NODE_INDEX"]),
];

/// Per-host env vars identifying the current CI run, for the de-dup file scope.
/// Distinct runs must not share a collision counter; entries are joined with `-`.
const CI_RUN_SCOPE_VARS: &[(&str, &[&str])] = &[
    ("GITHUB_ACTIONS", &["GITHUB_RUN_ID", "GITHUB_RUN_ATTEMPT"]),
    ("BUILDKITE", &["BUILDKITE_BUILD_ID"]),
    ("CIRCLECI", &["CIRCLE_WORKFLOW_ID"]),
    ("GITLAB_CI", &["CI_PIPELINE_ID"]),
];

/// The recognized CI job name, sanitized to the `--task:name` charset, or `None`
/// off CI / when no job-name var is set.
pub fn detect_ci_job_name() -> Option<String> {
    ci_job_name(|v| std::env::var(v).ok())
}

/// Pure core of [`detect_ci_job_name`], env-injected for testing.
///
/// `<job>` from [`CI_JOB_NAME_VARS`], with a `-<shard>` suffix appended when the
/// step uses native parallelism ([`CI_SHARD_INDEX_VARS`]) so each shard gets a
/// distinct name (they run on separate agents and can't share the de-dup file).
fn ci_job_name(get: impl Fn(&str) -> Option<String>) -> Option<String> {
    let job = first_present_value(CI_JOB_NAME_VARS, &get).and_then(|raw| sanitize_name(&raw))?;
    match first_present_value(CI_SHARD_INDEX_VARS, &get).and_then(|raw| sanitize_name(&raw)) {
        Some(shard) => Some(format!("{job}-{shard}")),
        None => Some(job),
    }
}

/// First non-empty value across the var-lists of the first present host.
/// A host is "present" iff its marker var is set (even to empty).
fn first_present_value(
    table: &[(&str, &[&str])],
    get: &impl Fn(&str) -> Option<String>,
) -> Option<String> {
    for (marker, vars) in table {
        if get(marker).is_none() {
            continue;
        }
        for var in *vars {
            if let Some(val) = get(var)
                && !val.trim().is_empty()
            {
                return Some(val);
            }
        }
        // Host matched but exposed no job name → stop; don't fall through to
        // another host's vars (we're on this host).
        return None;
    }
    None
}

/// Reduce an arbitrary CI job label to the `--task:name` charset
/// (`[A-Za-z0-9_-]`): any other run of characters collapses to a single `-`,
/// then leading/trailing `-` are trimmed. `None` if nothing survives.
fn sanitize_name(raw: &str) -> Option<String> {
    let mut out = String::with_capacity(raw.len());
    let mut pending_dash = false;
    for c in raw.chars() {
        if c.is_ascii_alphanumeric() || c == '_' {
            if pending_dash && !out.is_empty() {
                out.push('-');
            }
            pending_dash = false;
            out.push(c);
        } else {
            // `-` and every other separator collapse to one dash.
            pending_dash = true;
        }
    }
    if out.is_empty() { None } else { Some(out) }
}

/// Identifier for the current CI run, sanitized for use as a directory name,
/// or `None` off CI / when no run var is set.
fn run_scope(get: impl Fn(&str) -> Option<String>) -> Option<String> {
    for (marker, vars) in CI_RUN_SCOPE_VARS {
        if get(marker).is_none() {
            continue;
        }
        let parts: Vec<String> = vars
            .iter()
            .filter_map(|v| get(v))
            .filter(|s| !s.trim().is_empty())
            .collect();
        let joined = parts.join("-");
        return sanitize_name(&joined);
    }
    None
}

/// Directory that scopes the per-run task-name de-dup files. Lives under the
/// job tmpdir (Aspect Workflows / GitHub `RUNNER_TEMP` / `TMPDIR` / OS temp),
/// in a `<run-scope>` subdir so distinct runs don't share counters.
fn dedup_dir(get: impl Fn(&str) -> Option<String>) -> PathBuf {
    let base = get("ASPECT_WORKFLOWS_RUNNER_JOB_TMPDIR")
        .filter(|s| !s.is_empty())
        .or_else(|| get("RUNNER_TEMP").filter(|s| !s.is_empty()))
        .or_else(|| get("TMPDIR").filter(|s| !s.is_empty()))
        .map(PathBuf::from)
        .unwrap_or_else(std::env::temp_dir);
    let scope = run_scope(&get).unwrap_or_else(|| "local".to_string());
    base.join("aspect-task-names").join(scope)
}

/// Hard cap on collision-counter probing — far above any real pipeline; a
/// backstop against an unbounded loop if the filesystem misbehaves.
const MAX_NAME_PROBE: usize = 1000;

/// Reserve a unique task name derived from `base` (`<kind>-<job>`), appending
/// `-2`, `-3`, … on collision. Returns the first candidate this process wins.
///
/// Resolution is via an atomic claim file under the per-run de-dup directory:
/// `OpenOptions::create_new` (O_EXCL) lets the first process to create a given
/// candidate's marker win it, with no advisory locking. Any filesystem error
/// degrades to returning `base` unchanged — auto-naming must never fail a task.
pub fn reserve_task_name_in_tmpdir(base: &str) -> String {
    let dir = dedup_dir(|v| std::env::var(v).ok());
    if std::fs::create_dir_all(&dir).is_err() {
        return base.to_owned();
    }
    reserve_task_name(base, |candidate| {
        // Claim succeeds iff we created the marker; AlreadyExists → taken.
        match std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(dir.join(candidate))
        {
            Ok(_) => ClaimResult::Won,
            Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => ClaimResult::Taken,
            Err(_) => ClaimResult::Error,
        }
    })
}

/// Outcome of attempting to claim a candidate name.
#[derive(Debug, PartialEq, Eq)]
enum ClaimResult {
    /// This process now owns the candidate.
    Won,
    /// Another invocation already owns it; try the next suffix.
    Taken,
    /// Filesystem error; give up and use `base` unchanged.
    Error,
}

/// Pure core of [`reserve_task_name_in_tmpdir`], parameterized over the claim
/// operation so the suffixing/collision logic is testable without a filesystem.
fn reserve_task_name(base: &str, claim: impl Fn(&str) -> ClaimResult) -> String {
    for n in 1..=MAX_NAME_PROBE {
        let candidate = if n == 1 {
            base.to_owned()
        } else {
            format!("{base}-{n}")
        };
        match claim(&candidate) {
            ClaimResult::Won => return candidate,
            ClaimResult::Taken => continue,
            ClaimResult::Error => break,
        }
    }
    base.to_owned()
}

/// An auto-generated task name plus whether it carries real meaning.
///
/// `meaningful` is true when the name derives from the CI job (worth surfacing
/// next to the kind on status lines), false for the local random fallback (a
/// throwaway placeholder that reads as noise — callers show just the kind).
pub struct AutoTaskName {
    pub name: String,
    pub meaningful: bool,
}

/// The auto-generated task name for `kind` when `--task:name` is unset.
///
/// On a recognized CI host with a detectable job name: `<kind>-<job>`, made
/// unique on the local machine via [`reserve_task_name_in_tmpdir`] (meaningful).
/// Otherwise `<kind>-<fallback_suffix>` (the caller supplies a random friendly
/// suffix) — unique, no de-dup file, but a throwaway placeholder.
pub fn auto_task_name(kind: &str, fallback_suffix: impl FnOnce() -> String) -> AutoTaskName {
    match detect_ci_job_name() {
        Some(job) => AutoTaskName {
            name: reserve_task_name_in_tmpdir(&format!("{kind}-{job}")),
            meaningful: true,
        },
        None => AutoTaskName {
            name: format!("{kind}-{}", fallback_suffix()),
            meaningful: false,
        },
    }
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

    /// Build an env lookup from `(var, value)` pairs for the pure cores.
    fn env<'a>(pairs: &'a [(&'a str, &'a str)]) -> impl Fn(&str) -> Option<String> + 'a {
        move |var| {
            pairs
                .iter()
                .find(|(k, _)| *k == var)
                .map(|(_, v)| v.to_string())
        }
    }

    #[test]
    fn ci_job_name_per_host() {
        assert_eq!(
            ci_job_name(env(&[
                ("GITHUB_ACTIONS", "true"),
                ("GITHUB_JOB", "ci-linux")
            ])),
            Some("ci-linux".to_string())
        );
        assert_eq!(
            ci_job_name(env(&[("CIRCLECI", "true"), ("CIRCLE_JOB", "build")])),
            Some("build".to_string())
        );
        assert_eq!(
            ci_job_name(env(&[("GITLAB_CI", "true"), ("CI_JOB_NAME", "test")])),
            Some("test".to_string())
        );
    }

    #[test]
    fn ci_job_name_buildkite_prefers_step_key_over_label() {
        assert_eq!(
            ci_job_name(env(&[
                ("BUILDKITE", "true"),
                ("BUILDKITE_STEP_KEY", "unit"),
                ("BUILDKITE_LABEL", ":hammer: Unit tests"),
            ])),
            Some("unit".to_string())
        );
        // Falls back to the label when no step key is set.
        assert_eq!(
            ci_job_name(env(&[
                ("BUILDKITE", "true"),
                ("BUILDKITE_LABEL", ":hammer: Unit tests"),
            ])),
            Some("hammer-Unit-tests".to_string())
        );
    }

    #[test]
    fn ci_job_name_none_off_ci_or_empty() {
        assert_eq!(ci_job_name(env(&[])), None);
        // Host present but no job var → None (don't fall through to another host).
        assert_eq!(ci_job_name(env(&[("GITHUB_ACTIONS", "true")])), None);
        // Whitespace-only job var → None.
        assert_eq!(
            ci_job_name(env(&[("GITHUB_ACTIONS", "true"), ("GITHUB_JOB", "  ")])),
            None
        );
    }

    #[test]
    fn ci_job_name_appends_parallel_shard_index() {
        // Buildkite parallelism: BUILDKITE_PARALLEL_JOB disambiguates shards.
        assert_eq!(
            ci_job_name(env(&[
                ("BUILDKITE", "true"),
                ("BUILDKITE_STEP_KEY", "unit"),
                ("BUILDKITE_PARALLEL_JOB", "0"),
            ])),
            Some("unit-0".to_string())
        );
        // CircleCI parallelism: CIRCLE_NODE_INDEX.
        assert_eq!(
            ci_job_name(env(&[
                ("CIRCLECI", "true"),
                ("CIRCLE_JOB", "test"),
                ("CIRCLE_NODE_INDEX", "3"),
            ])),
            Some("test-3".to_string())
        );
        // No parallelism var → bare job name (non-parallel steps are unaffected).
        assert_eq!(
            ci_job_name(env(&[("CIRCLECI", "true"), ("CIRCLE_JOB", "test")])),
            Some("test".to_string())
        );
        // GitHub matrix has no shard env var, so the job name stays bare.
        assert_eq!(
            ci_job_name(env(&[
                ("GITHUB_ACTIONS", "true"),
                ("GITHUB_JOB", "ci-linux")
            ])),
            Some("ci-linux".to_string())
        );
    }

    #[test]
    fn sanitize_name_collapses_separators() {
        assert_eq!(sanitize_name("ci-linux"), Some("ci-linux".to_string()));
        assert_eq!(
            sanitize_name("Build (linux, x64)"),
            Some("Build-linux-x64".to_string())
        );
        assert_eq!(
            sanitize_name("  leading/trailing  "),
            Some("leading-trailing".to_string())
        );
        assert_eq!(
            sanitize_name("keeps_underscore"),
            Some("keeps_underscore".to_string())
        );
        assert_eq!(sanitize_name(""), None);
        assert_eq!(sanitize_name("/// ---"), None);
    }

    #[test]
    fn run_scope_per_host() {
        assert_eq!(
            run_scope(env(&[
                ("GITHUB_ACTIONS", "true"),
                ("GITHUB_RUN_ID", "42"),
                ("GITHUB_RUN_ATTEMPT", "2"),
            ])),
            Some("42-2".to_string())
        );
        assert_eq!(
            run_scope(env(&[("CIRCLECI", "true"), ("CIRCLE_WORKFLOW_ID", "wf-9")])),
            Some("wf-9".to_string())
        );
        assert_eq!(run_scope(env(&[])), None);
    }

    #[test]
    fn reserve_task_name_suffixes_on_collision() {
        use std::cell::RefCell;
        use std::collections::HashSet;

        // First-claim-wins over an in-memory set of taken names.
        let taken: RefCell<HashSet<String>> = RefCell::new(HashSet::new());
        let claim = |c: &str| {
            if taken.borrow().contains(c) {
                ClaimResult::Taken
            } else {
                taken.borrow_mut().insert(c.to_string());
                ClaimResult::Won
            }
        };

        assert_eq!(reserve_task_name("test-ci", &claim), "test-ci");
        assert_eq!(reserve_task_name("test-ci", &claim), "test-ci-2");
        assert_eq!(reserve_task_name("test-ci", &claim), "test-ci-3");
        // A different base is independent.
        assert_eq!(reserve_task_name("build-ci", &claim), "build-ci");
    }

    #[test]
    fn reserve_task_name_degrades_to_base_on_error() {
        assert_eq!(
            reserve_task_name("test-ci", |_| ClaimResult::Error),
            "test-ci"
        );
    }

    #[test]
    fn auto_task_name_prefixes_kind() {
        // `detect_ci_job_name` reads the real env, so the `meaningful` flag
        // depends on whether the test runs on CI; the `<kind>-…` shape holds
        // in both branches.
        let auto = auto_task_name("greet", || "fluffy-parakeet".to_string());
        assert!(auto.name.starts_with("greet-"), "got {}", auto.name);
    }
}
