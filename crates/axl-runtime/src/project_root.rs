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
const ASPECT_BOUNDARY_FILES: &[&str] = &[AXL_MODULE_FILE, ".aspect/version.axl"];

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
fn find_root_with_fallback(start: &Path, primary: &[&str], fallback: &[&str]) -> Option<PathBuf> {
    find_ancestor_with_any(start, primary).or_else(|| find_ancestor_with_any(start, fallback))
}

/// The deepest ancestor of `start` (itself included) holding any of `markers`.
fn find_ancestor_with_any(start: &Path, markers: &[&str]) -> Option<PathBuf> {
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
    use tempfile::{TempDir, tempdir};

    /// A temp directory holding `layout`, each entry a path relative to its
    /// root, written empty. Returns the handle (kept alive for the test) and
    /// the root.
    fn setup(layout: &[&str]) -> (TempDir, PathBuf) {
        let tmp = tempdir().unwrap();
        let root = tmp.path().to_path_buf();
        for rel in layout {
            let path = root.join(rel);
            if let Some(parent) = path.parent() {
                std::fs::create_dir_all(parent).unwrap();
            }
            std::fs::write(&path, "").unwrap();
        }
        (tmp, root)
    }

    fn subdir(root: &Path, rel: &str) -> PathBuf {
        let dir = root.join(rel);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    /// Aspect marker outside, Bazel marker in a sub-workspace: the two roots
    /// diverge at exactly the boundary their consumers care about.
    #[test]
    fn aspect_and_bazel_roots_diverge_in_sub_workspace() {
        let (_tmp, root) = setup(&[".aspect/version.axl", "e2e/MODULE.bazel"]);
        let cwd = subdir(&root, "e2e/sub");

        assert_eq!(find_aspect_root(&cwd), Some(root.clone()));
        assert_eq!(find_bazel_root(&cwd), Some(root.join("e2e")));
    }

    /// Pure-Aspect workspace → both roots resolve to the Aspect marker.
    #[test]
    fn bazel_root_falls_back_to_aspect_marker() {
        let (_tmp, root) = setup(&[".aspect/version.axl"]);
        let cwd = subdir(&root, "sub");

        assert_eq!(find_aspect_root(&cwd), Some(root.clone()));
        assert_eq!(find_bazel_root(&cwd), Some(root));
    }

    /// Pure-Bazel monorepo → both roots resolve to the Bazel marker.
    #[test]
    fn aspect_root_falls_back_to_bazel_marker() {
        let (_tmp, root) = setup(&["MODULE.bazel"]);
        let cwd = subdir(&root, "sub");

        assert_eq!(find_aspect_root(&cwd), Some(root.clone()));
        assert_eq!(find_bazel_root(&cwd), Some(root));
    }

    /// No markers anywhere → both roots are `None`; callers supply their own
    /// fallback (typically cwd).
    #[test]
    fn both_roots_are_none_when_no_markers() {
        let (_tmp, root) = setup(&[]);
        let cwd = subdir(&root, "sub");

        assert_eq!(find_aspect_root(&cwd), None);
        assert_eq!(find_bazel_root(&cwd), None);
    }

    /// `MODULE.aspect` is recognized as an Aspect marker.
    #[test]
    fn aspect_root_recognizes_module_aspect() {
        let (_tmp, root) = setup(&["MODULE.aspect"]);
        let cwd = subdir(&root, "sub");

        assert_eq!(find_aspect_root(&cwd), Some(root.clone()));
        assert_eq!(find_bazel_root(&cwd), Some(root));
    }

    /// Every documented Bazel marker is recognized — guards against silent
    /// drift in `BAZEL_BOUNDARY_FILES`.
    #[test]
    fn bazel_root_recognizes_every_marker() {
        for marker in BAZEL_BOUNDARY_FILES {
            let (_tmp, root) = setup(&[marker]);
            let cwd = subdir(&root, "sub");
            assert_eq!(find_bazel_root(&cwd), Some(root), "marker: {marker}");
        }
    }

    /// `.git` directory found by walking up from a subdirectory.
    #[test]
    fn git_root_found_from_subdirectory() {
        let (_tmp, root) = setup(&[]);
        std::fs::create_dir_all(root.join(".git")).unwrap();
        let cwd = subdir(&root, "a/b/c");

        assert_eq!(find_git_root(&cwd), Some(root));
    }

    /// `.git` file (git worktree) is recognized.
    #[test]
    fn git_root_recognizes_worktree_git_file() {
        let (_tmp, root) = setup(&[".git"]);
        let cwd = subdir(&root, "sub");

        assert_eq!(find_git_root(&cwd), Some(root));
    }

    /// Returns `None` when not inside any git repository.
    #[test]
    fn git_root_none_outside_repo() {
        let (_tmp, root) = setup(&[]);
        let cwd = subdir(&root, "sub");

        assert_eq!(find_git_root(&cwd), None);
    }

    /// The git root is the outermost ancestor holding `.git`, independent of
    /// where the Bazel and Aspect roots land.
    #[test]
    fn git_root_is_independent_of_bazel_and_aspect_roots() {
        let (_tmp, root) = setup(&[".git", "e2e/MODULE.bazel"]);
        let cwd = subdir(&root, "e2e/sub");

        assert_eq!(find_git_root(&cwd), Some(root.clone()));
        assert_eq!(find_bazel_root(&cwd), Some(root.join("e2e")));
    }

    /// The directory the caller passes counts as its own ancestor, so a marker
    /// sitting in it resolves there rather than above it.
    #[test]
    fn the_starting_directory_is_its_own_root() {
        let (_tmp, root) = setup(&[".aspect/version.axl"]);

        assert_eq!(find_aspect_root(&root), Some(root));
    }
}
