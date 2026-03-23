use std::collections::HashSet;
use std::path::{Path, PathBuf};

use crate::BazelRcError;

/// Discover the ordered list of bazelrc files to load.
pub(crate) fn discover_rc_files(
    workspace_root: &Path,
    startup_flags: &[impl AsRef<str>],
) -> Result<Vec<PathBuf>, BazelRcError> {
    let mut no_system_rc = false;
    let mut no_workspace_rc = false;
    let mut no_home_rc = false;
    let mut ignore_all_rc = false;
    let mut explicit_bazelrcs: Vec<PathBuf> = Vec::new();

    for flag in startup_flags {
        let s = flag.as_ref();
        match s {
            "--nosystem_rc" => no_system_rc = true,
            "--noworkspace_rc" => no_workspace_rc = true,
            "--nohome_rc" => no_home_rc = true,
            "--ignore_all_rc_files" => ignore_all_rc = true,
            _ => {
                if let Some(path) = s.strip_prefix("--bazelrc=") {
                    explicit_bazelrcs.push(PathBuf::from(path));
                }
            }
        }
    }

    let mut candidates: Vec<PathBuf> = Vec::new();

    if !ignore_all_rc {
        // System RC
        if !no_system_rc {
            let system_rc = system_rc_path();
            if system_rc.exists() {
                candidates.push(system_rc);
            }
        }

        // Workspace RC
        if !no_workspace_rc {
            let workspace_rc = workspace_root.join(".bazelrc");
            if workspace_rc.exists() {
                candidates.push(workspace_rc);
            }
        }

        // Home RC
        if !no_home_rc {
            if let Some(home_rc) = home_rc_path() {
                if home_rc.exists() {
                    candidates.push(home_rc);
                }
            }
        }

        // BAZELRC env var (comma-separated)
        if let Ok(env_val) = std::env::var("BAZELRC") {
            for part in env_val.split(',') {
                let p = PathBuf::from(part.trim());
                if !p.as_os_str().is_empty() {
                    if !p.exists() {
                        return Err(BazelRcError::BazelrcNotFound { path: p });
                    }
                    candidates.push(p);
                }
            }
        }
    }

    // Explicit --bazelrc= files are suppressed by --ignore_all_rc_files (Bazel spec).
    if !ignore_all_rc {
        for p in explicit_bazelrcs {
            if !p.exists() {
                return Err(BazelRcError::BazelrcNotFound { path: p });
            }
            candidates.push(p);
        }
    }

    // Canonicalize and deduplicate while preserving order
    let mut seen: HashSet<PathBuf> = HashSet::new();
    let mut result: Vec<PathBuf> = Vec::new();
    for p in candidates {
        let canonical = p.canonicalize().unwrap_or(p);
        if seen.insert(canonical.clone()) {
            result.push(canonical);
        }
    }

    Ok(result)
}

#[cfg(target_os = "windows")]
fn system_rc_path() -> PathBuf {
    let base = std::env::var("PROGRAMDATA").unwrap_or_else(|_| r"C:\ProgramData".to_owned());
    PathBuf::from(base).join("bazel.bazelrc")
}

#[cfg(not(target_os = "windows"))]
fn system_rc_path() -> PathBuf {
    PathBuf::from("/etc/bazel.bazelrc")
}

#[cfg(target_os = "windows")]
fn home_rc_path() -> Option<PathBuf> {
    let home = std::env::var("USERPROFILE").ok()?;
    Some(PathBuf::from(home).join(".bazelrc"))
}

#[cfg(not(target_os = "windows"))]
fn home_rc_path() -> Option<PathBuf> {
    let home = std::env::var("HOME").ok()?;
    Some(PathBuf::from(home).join(".bazelrc"))
}
