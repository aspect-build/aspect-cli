//! Where a project starts: the Aspect root, the Bazel workspace root, and the
//! git root, found by walking up from a directory looking for the file each is
//! marked by.
//!
//! One implementation, because more than one part of the CLI needs an answer and
//! they have to agree. The CLI resolves all three once at startup, hands them to
//! AXL as `ctx.std.env.aspect_root_dir()` / `.bazel_root_dir()` /
//! `.git_root_dir()`, and records the Aspect root for code with no evaluator to
//! read it from (`engine::store::resolved_aspect_root`). Walking again is for
//! the one process that resolves nothing at startup: the `aspect get` credential
//! helper, which Bazel spawns as a bare subprocess.
//!
//! Synchronous on purpose: these are a handful of `stat`s up a path, and the
//! credential helper runs before the CLI starts its async runtime.

use std::path::{Path, PathBuf};

use crate::module::AXL_MODULE_FILE;

/// Conventional name of the `.aspect` directory under an Aspect project root.
pub const DOT_ASPECT_FOLDER: &str = ".aspect";

/// Markers identifying an Aspect project root.
pub const ASPECT_BOUNDARY_FILES: &[&str] = &[AXL_MODULE_FILE, ".aspect/version.axl"];

/// Markers identifying a Bazel workspace root (see
/// <https://bazel.build/external/overview#repository>).
pub const BAZEL_BOUNDARY_FILES: &[&str] = &[
    "MODULE.bazel",
    "MODULE.bazel.lock",
    "REPO.bazel",
    "WORKSPACE",
    "WORKSPACE.bazel",
];

/// Aspect project root for axl / config loading.
///
/// Deepest ancestor of `start` containing `.aspect/version.axl` or
/// `MODULE.aspect`. Falls back to the deepest Bazel workspace marker so a
/// pure-Bazel monorepo still resolves to a sane project anchor. `None` only when
/// neither marker exists anywhere in the ancestry.
pub fn find_aspect_root(start: &Path) -> Option<PathBuf> {
    find_root_with_fallback(start, ASPECT_BOUNDARY_FILES, BAZEL_BOUNDARY_FILES)
}

/// The Aspect project root of the current working directory, for a caller with
/// no directory of its own to start from.
pub fn aspect_root_from_cwd() -> Option<PathBuf> {
    find_aspect_root(&std::env::current_dir().ok()?)
}

/// Bazel workspace root for bazelrc discovery, `bazel info workspace`, and BES
/// output paths.
///
/// Deepest ancestor of `start` containing a Bazel marker, falling back to the
/// deepest Aspect marker so a pure-Aspect workspace still resolves.
///
/// Diverges from [`find_aspect_root`] when both markers exist in the ancestry:
/// with `/proj/.aspect/version.axl` and `/proj/e2e/MODULE.bazel`, starting from
/// `/proj/e2e/sub/` puts the Aspect root at `/proj` and the Bazel root at
/// `/proj/e2e`.
pub fn find_bazel_root(start: &Path) -> Option<PathBuf> {
    find_root_with_fallback(start, BAZEL_BOUNDARY_FILES, ASPECT_BOUNDARY_FILES)
}

/// Git repository root — the directory holding the `.git` entry, which is a
/// directory in a normal clone and a file in a worktree. `None` outside a
/// repository.
pub fn find_git_root(start: &Path) -> Option<PathBuf> {
    find_ancestor_with_any(start, &[".git"])
}

/// Walk ancestors of `start` for `primary`; on a miss, walk again for
/// `fallback`. Two passes rather than one, so the *shallower* primary marker
/// still beats a deeper fallback.
pub fn find_root_with_fallback(
    start: &Path,
    primary: &[&str],
    fallback: &[&str],
) -> Option<PathBuf> {
    find_ancestor_with_any(start, primary).or_else(|| find_ancestor_with_any(start, fallback))
}

/// The deepest ancestor of `start` (itself included) holding any of `markers`.
pub fn find_ancestor_with_any(start: &Path, markers: &[&str]) -> Option<PathBuf> {
    start
        .ancestors()
        .find(|ancestor| {
            markers
                .iter()
                .any(|marker| ancestor.join(marker).try_exists().unwrap_or(false))
        })
        .map(Path::to_path_buf)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn touch(path: &Path) {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).unwrap();
        }
        std::fs::write(path, "").unwrap();
    }

    #[test]
    fn the_aspect_root_is_the_nearest_marked_ancestor() {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path();
        touch(&root.join(".aspect/version.axl"));
        let deep = root.join("a/b/c");
        std::fs::create_dir_all(&deep).unwrap();

        assert_eq!(find_aspect_root(&deep).as_deref(), Some(root));
        assert_eq!(find_aspect_root(root).as_deref(), Some(root));
    }

    #[test]
    fn a_pure_bazel_repo_falls_back_to_its_workspace_marker() {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path();
        touch(&root.join("MODULE.bazel"));

        assert_eq!(find_aspect_root(root).as_deref(), Some(root));
    }

    /// The two roots part company when a Bazel sub-workspace sits under an
    /// Aspect root, which is the case the fallback ordering exists for.
    #[test]
    fn a_bazel_sub_workspace_keeps_its_own_root() {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path();
        touch(&root.join(".aspect/version.axl"));
        let sub = root.join("e2e");
        touch(&sub.join("MODULE.bazel"));

        assert_eq!(find_aspect_root(&sub).as_deref(), Some(root));
        assert_eq!(find_bazel_root(&sub).as_deref(), Some(sub.as_path()));
    }

    #[test]
    fn an_unmarked_ancestry_has_no_root() {
        let tmp = tempfile::tempdir().unwrap();
        let deep = tmp.path().join("a/b");
        std::fs::create_dir_all(&deep).unwrap();

        assert_eq!(find_ancestor_with_any(&deep, &["no-such-marker"]), None);
    }
}
