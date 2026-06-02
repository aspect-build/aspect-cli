//! Detect and recover from runner-poisoning sandbox state described by
//! bazelbuild/bazel#23880.
//!
//! When `LinuxSandboxedStrategy.create` throws `IOException` (e.g. an
//! `fchmod` rejected by the filesystem during sandbox setup), the
//! `linux-sandbox` SpawnRunner is never registered. Bazel's
//! `SandboxModule.afterCommand` cleanup loop iterates the registered
//! runners only, so the partially-set-up `<output_base>/sandbox/linux-sandbox`
//! subtree is never deleted. The follow-up
//! `checkSandboxBaseTopOnlyContainsPersistentDirs` precondition then
//! crashes the command. The on-disk subtree survives the crash and
//! poisons every subsequent bazel invocation on the same output_base
//! the same way.
//!
//! Bazel's whitelist of allowed entries directly under the sandbox base
//! (`SANDBOX_BASE_PERSISTENT_DIRS`) is `{.DS_Store, sandbox_stash,
//! sandbox_stash_temp, _moved_trash_dir}`. Anything else present after
//! a bazel command exited is the poisoning signature.
//!
//! Fixed upstream by `abe8d6090` (Bazel 9.0+); 8.x users hit this until
//! they upgrade. This module is the runner-side mitigation: detect the
//! state post-invocation, attempt to remove the offending entries, and
//! report whether anything remained so the caller can mark the runner
//! unhealthy.

use std::path::{Path, PathBuf};

use allocative::Allocative;
use derive_more::Display;
use starlark::environment::{Methods, MethodsBuilder, MethodsStatic};
use starlark::starlark_module;
use starlark::starlark_simple_value;
use starlark::values;
use starlark::values::starlark_value;
use starlark::values::{NoSerialize, ProvidesStaticType, ValueLike};

/// Entries Bazel itself considers legitimate at the top level of the
/// sandbox base. Mirrors `SandboxModule.SANDBOX_BASE_PERSISTENT_DIRS`
/// in the Bazel source. Any other entry present after a bazel command
/// has exited indicates the runner is in the bazel#23880 state.
const SANDBOX_BASE_PERSISTENT_DIRS: &[&str] = &[
    ".DS_Store",
    "sandbox_stash",
    "sandbox_stash_temp",
    "_moved_trash_dir",
];

/// Outcome of a sandbox-recovery attempt. Drives the caller's decision
/// to either log + continue (`Clean` / `Repaired`) or signal the runner
/// unhealthy (`StillPoisoned`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RecoveryOutcome {
    /// `<output_base>/sandbox/` either doesn't exist or contains only
    /// whitelisted entries. The runner is healthy.
    Clean,
    /// Non-whitelisted entries were present and successfully removed.
    /// Caller should log the recovery; the runner can serve the next job.
    Repaired { removed: Vec<String> },
    /// Non-whitelisted entries were present and at least one could not
    /// be removed. Caller should signal the runner unhealthy so the
    /// next job lands on a fresh instance.
    StillPoisoned {
        removed: Vec<String>,
        remaining: Vec<String>,
    },
}

impl RecoveryOutcome {
    /// Starlark-facing outcome tag.
    fn tag(&self) -> &'static str {
        match self {
            RecoveryOutcome::Clean => "clean",
            RecoveryOutcome::Repaired { .. } => "repaired",
            RecoveryOutcome::StillPoisoned { .. } => "still_poisoned",
        }
    }

    fn removed_entries(&self) -> Vec<String> {
        match self {
            RecoveryOutcome::Clean => Vec::new(),
            RecoveryOutcome::Repaired { removed } => removed.clone(),
            RecoveryOutcome::StillPoisoned { removed, .. } => removed.clone(),
        }
    }

    fn remaining_entries(&self) -> Vec<String> {
        match self {
            RecoveryOutcome::Clean | RecoveryOutcome::Repaired { .. } => Vec::new(),
            RecoveryOutcome::StillPoisoned { remaining, .. } => remaining.clone(),
        }
    }
}

/// Starlark-facing wrapper around [`RecoveryOutcome`]. Returned by
/// `ctx.bazel.recover_poisoned_sandbox()`.
#[derive(Debug, Display, ProvidesStaticType, NoSerialize, Allocative)]
#[display("<bazel.SandboxRecoveryResult>")]
pub struct SandboxRecoveryResult {
    outcome: String,
    removed: Vec<String>,
    remaining: Vec<String>,
}

impl SandboxRecoveryResult {
    pub(crate) fn from_outcome(outcome: RecoveryOutcome) -> Self {
        Self {
            outcome: outcome.tag().to_string(),
            removed: outcome.removed_entries(),
            remaining: outcome.remaining_entries(),
        }
    }

    /// Builder used when the caller skipped recovery (e.g. no
    /// `--output_base` in startup flags). Reads as `clean` so callers
    /// can branch uniformly on the outcome tag.
    pub(crate) fn skipped() -> Self {
        Self {
            outcome: "clean".to_string(),
            removed: Vec::new(),
            remaining: Vec::new(),
        }
    }
}

starlark_simple_value!(SandboxRecoveryResult);

#[starlark_value(type = "bazel.SandboxRecoveryResult")]
impl<'v> values::StarlarkValue<'v> for SandboxRecoveryResult {
    fn get_methods() -> Option<&'static Methods> {
        static RES: MethodsStatic = MethodsStatic::new();
        RES.methods(sandbox_recovery_result_methods)
    }
}

#[starlark_module]
pub(crate) fn sandbox_recovery_result_methods(registry: &mut MethodsBuilder) {
    /// Recovery outcome: `"clean"`, `"repaired"`, or `"still_poisoned"`.
    ///
    /// - `clean`: nothing to do. Either the sandbox dir didn't exist or
    ///   contained only whitelisted entries.
    /// - `repaired`: non-whitelisted entries were found and successfully
    ///   removed. The runner is usable for the next job.
    /// - `still_poisoned`: non-whitelisted entries were found and at least
    ///   one could not be removed. Caller should signal the runner
    ///   unhealthy.
    #[starlark(attribute)]
    fn outcome<'v>(this: values::Value<'v>) -> anyhow::Result<String> {
        Ok(this
            .downcast_ref::<SandboxRecoveryResult>()
            .unwrap()
            .outcome
            .clone())
    }

    /// Sorted names of sandbox-base entries that were successfully removed.
    /// Empty when `outcome == "clean"`.
    #[starlark(attribute)]
    fn removed<'v>(this: values::Value<'v>) -> anyhow::Result<Vec<String>> {
        Ok(this
            .downcast_ref::<SandboxRecoveryResult>()
            .unwrap()
            .removed
            .clone())
    }

    /// Sorted names of sandbox-base entries that survived the removal
    /// attempt. Only non-empty when `outcome == "still_poisoned"`.
    #[starlark(attribute)]
    fn remaining<'v>(this: values::Value<'v>) -> anyhow::Result<Vec<String>> {
        Ok(this
            .downcast_ref::<SandboxRecoveryResult>()
            .unwrap()
            .remaining
            .clone())
    }
}

/// Snapshot non-whitelisted entries directly under `<output_base>/sandbox/`.
///
/// Returns the entries' file names (not full paths) sorted lexicographically
/// for deterministic logging. An absent sandbox dir (fresh output_base, or
/// `output_base` itself missing) returns an empty vector — not an error.
fn list_unexpected_entries(output_base: &Path) -> Vec<String> {
    let sandbox = output_base.join("sandbox");
    let Ok(entries) = std::fs::read_dir(&sandbox) else {
        return Vec::new();
    };
    let mut out: Vec<String> = entries
        .filter_map(Result::ok)
        .filter_map(|e| e.file_name().into_string().ok())
        .filter(|name| !SANDBOX_BASE_PERSISTENT_DIRS.contains(&name.as_str()))
        .collect();
    out.sort();
    out
}

/// Detect and (best-effort) repair bazel#23880-style sandbox poisoning.
///
/// Lists unexpected entries under `<output_base>/sandbox/`, then attempts
/// to remove each one. Re-lists afterwards to report what (if anything)
/// survived. Safe to call only AFTER the bazel client invocation has
/// fully exited — a live `bazel build/test` would race the rm.
///
/// Each removal goes through `symlink_metadata` first so symlinks are
/// inspected, not followed; then either `remove_dir_all` (for directories)
/// or `remove_file` (for files / symlinks). A removal that fails is
/// logged as a warning and the entry is left for the caller to handle
/// via `signal_instance_unhealthy`.
pub fn recover(output_base: &Path) -> RecoveryOutcome {
    let initial = list_unexpected_entries(output_base);
    if initial.is_empty() {
        return RecoveryOutcome::Clean;
    }
    let sandbox = output_base.join("sandbox");
    let mut removed = Vec::new();
    for name in &initial {
        let path = sandbox.join(name);
        let res = match std::fs::symlink_metadata(&path) {
            Ok(meta) if meta.file_type().is_dir() => std::fs::remove_dir_all(&path),
            Ok(_) => std::fs::remove_file(&path),
            Err(e) => Err(e),
        };
        match res {
            Ok(()) => {
                tracing::warn!(
                    path = %path.display(),
                    "Removed bazel#23880-poisoning sandbox entry left by a previous \
                     bazel invocation"
                );
                removed.push(name.clone());
            }
            Err(e) => {
                tracing::warn!(
                    path = %path.display(),
                    error = %e,
                    "Failed to remove bazel#23880-poisoning sandbox entry — the runner \
                     will be marked unhealthy",
                );
            }
        }
    }
    let remaining = list_unexpected_entries(output_base);
    if remaining.is_empty() {
        RecoveryOutcome::Repaired { removed }
    } else {
        RecoveryOutcome::StillPoisoned { removed, remaining }
    }
}

/// Extract `--output_base=<path>` from the passed startup flags without
/// invoking bazel. Returns `None` when no `--output_base` flag is present
/// or its value is empty.
///
/// Same shape as the helper used by the health-check probe — kept local
/// to this module so the recovery path doesn't depend on a wedged
/// bazel server to resolve its own output_base.
pub fn output_base_from_flags(startup_flags: &[String]) -> Option<PathBuf> {
    for flag in startup_flags {
        if let Some(value) = flag.strip_prefix("--output_base=") {
            if !value.is_empty() {
                return Some(PathBuf::from(value));
            }
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_output_base() -> tempfile::TempDir {
        let dir = tempfile::tempdir().expect("tempdir");
        std::fs::create_dir(dir.path().join("sandbox")).expect("create sandbox dir");
        dir
    }

    #[test]
    fn clean_when_sandbox_dir_absent() {
        let base = tempfile::tempdir().expect("tempdir");
        assert_eq!(recover(base.path()), RecoveryOutcome::Clean);
    }

    #[test]
    fn clean_when_only_whitelisted_entries() {
        let base = make_output_base();
        for name in SANDBOX_BASE_PERSISTENT_DIRS {
            std::fs::create_dir(base.path().join("sandbox").join(name)).unwrap();
        }
        assert_eq!(recover(base.path()), RecoveryOutcome::Clean);
        // Whitelisted entries must survive the call.
        for name in SANDBOX_BASE_PERSISTENT_DIRS {
            assert!(
                base.path().join("sandbox").join(name).exists(),
                "whitelisted entry {} must survive recovery",
                name
            );
        }
    }

    #[test]
    fn repairs_linux_sandbox_subtree() {
        // The canonical bazel#23880 failure mode: a partial
        // `linux-sandbox/<pid>/...` tree survived a previous afterCommand.
        let base = make_output_base();
        let linux_sandbox = base.path().join("sandbox").join("linux-sandbox");
        std::fs::create_dir_all(linux_sandbox.join("73541").join("execroot")).unwrap();
        std::fs::write(
            linux_sandbox.join("73541").join("execroot").join("junk"),
            b"x",
        )
        .unwrap();

        let outcome = recover(base.path());
        assert_eq!(
            outcome,
            RecoveryOutcome::Repaired {
                removed: vec!["linux-sandbox".to_string()]
            }
        );
        assert!(!linux_sandbox.exists());
    }

    #[test]
    fn repairs_multiple_per_strategy_subtrees() {
        let base = make_output_base();
        for name in &["linux-sandbox", "processwrapper-sandbox", "darwin-sandbox"] {
            std::fs::create_dir(base.path().join("sandbox").join(name)).unwrap();
        }
        // Whitelisted entry alongside the unexpected ones should be preserved.
        std::fs::create_dir(base.path().join("sandbox").join("sandbox_stash")).unwrap();

        let outcome = recover(base.path());
        match outcome {
            RecoveryOutcome::Repaired { ref removed } => {
                let mut expected =
                    vec!["darwin-sandbox", "linux-sandbox", "processwrapper-sandbox"];
                expected.sort();
                let got: Vec<&str> = removed.iter().map(String::as_str).collect();
                assert_eq!(got, expected);
            }
            other => panic!("expected Repaired, got {:?}", other),
        }
        assert!(base.path().join("sandbox").join("sandbox_stash").exists());
    }

    #[cfg(unix)]
    fn chmod(path: &Path, mode: u32) {
        use std::os::unix::fs::PermissionsExt;
        let mut perms = std::fs::metadata(path).unwrap().permissions();
        perms.set_mode(mode);
        std::fs::set_permissions(path, perms).unwrap();
    }

    #[cfg(unix)]
    #[test]
    fn reports_still_poisoned_when_removal_fails() {
        // A read-only parent makes unlinkat/rmdir of the child fail with
        // EACCES — the case that drives a `still_poisoned` outcome.
        let base = make_output_base();
        let sandbox = base.path().join("sandbox");
        std::fs::create_dir(sandbox.join("linux-sandbox")).unwrap();

        chmod(&sandbox, 0o555);
        let outcome = recover(base.path());
        // Restore so the tempdir can be cleaned up.
        chmod(&sandbox, 0o755);

        assert!(
            matches!(outcome, RecoveryOutcome::StillPoisoned { .. }),
            "expected StillPoisoned, got {:?}",
            outcome
        );
    }

    #[test]
    fn output_base_from_flags_picks_explicit_flag() {
        let flags = vec![
            "--nohome_rc".to_string(),
            "--output_base=/mnt/ephemeral/output/repo".to_string(),
        ];
        assert_eq!(
            output_base_from_flags(&flags),
            Some(PathBuf::from("/mnt/ephemeral/output/repo"))
        );
    }

    #[test]
    fn output_base_from_flags_ignores_unrelated_prefix() {
        // `--output_user_root` shares the `--output_` prefix but is a
        // different flag — must not match.
        let flags = vec!["--output_user_root=/mnt/foo".to_string()];
        assert_eq!(output_base_from_flags(&flags), None);
    }

    #[test]
    fn output_base_from_flags_rejects_empty_value() {
        let flags = vec!["--output_base=".to_string()];
        assert_eq!(output_base_from_flags(&flags), None);
    }
}
