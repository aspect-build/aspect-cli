use std::path::{Path, PathBuf};

use axl_runtime::module::{AXL_CONFIG_EXTENSION, AXL_SCRIPT_EXTENSION, AXL_VERSION_EXTENSION};
use axl_runtime::project_root;
use tokio::fs;
use tracing::instrument;

/// Conventional name of the `.aspect` directory under an Aspect project root.
pub const DOT_ASPECT_FOLDER: &str = project_root::DOT_ASPECT_FOLDER;

/// Aspect project root for axl / config loading. The markers and the walk live
/// in [`project_root`], so every part of the CLI — this, and the deployment
/// config loader that runs without the task runtime — resolves the same root.
#[instrument]
pub async fn find_aspect_root(current_work_dir: &Path) -> Option<PathBuf> {
    project_root::find_aspect_root(current_work_dir)
}

/// Git repository root — see [`project_root::find_git_root`].
#[instrument]
pub async fn find_git_root(current_work_dir: &Path) -> Option<PathBuf> {
    project_root::find_git_root(current_work_dir)
}

/// Bazel workspace root for bazelrc discovery, `bazel info workspace`, and BES
/// output paths — see [`project_root::find_bazel_root`].
#[instrument]
pub async fn find_bazel_root(current_work_dir: &Path) -> Option<PathBuf> {
    project_root::find_bazel_root(current_work_dir)
}

/// User-global config file: `<home_dir>/.aspect/config.axl`, if it exists.
///
/// Callers append this after all project config.axl files so per-user
/// overrides are applied last among configs (explicit CLI flags still win
/// over any config value).
#[instrument]
pub async fn find_user_config(home_dir: Option<&Path>) -> Option<PathBuf> {
    let path = home_dir?.join(DOT_ASPECT_FOLDER).join(AXL_CONFIG_EXTENSION);
    match fs::metadata(&path).await {
        Ok(meta) if meta.is_file() => Some(path),
        _ => None,
    }
}

/// Returns a list of axl search paths by constructing paths from the
/// `aspect_root_dir` up to `current_work_dir`, appending `.aspect` to each.
/// If the relative path from `aspect_root_dir` to `current_work_dir` includes
/// a `.aspect` component, the search stops at the parent directory of that
/// `.aspect`, excluding `.aspect` and any subdirectories from the results.
#[instrument]
pub fn get_default_axl_search_paths(
    current_work_dir: &PathBuf,
    aspect_root_dir: &PathBuf,
) -> Vec<PathBuf> {
    if let Ok(rel_path) = current_work_dir.strip_prefix(aspect_root_dir) {
        let mut paths = vec![aspect_root_dir.join(DOT_ASPECT_FOLDER)];
        let mut current = aspect_root_dir.clone();
        for component in rel_path.components() {
            if component.as_os_str() == DOT_ASPECT_FOLDER {
                break;
            }
            current = current.join(component);
            paths.push(current.join(DOT_ASPECT_FOLDER));
        }
        paths
    } else {
        vec![]
    }
}

/// Asynchronously searches through the provided list of directories (`search_paths`) and collects
/// all files that have the extension matching `axl`.
/// For each directory, it checks if it exists and is a directory, then reads its entries and
/// filters for files with the specified extension.
/// Returns a vector of `PathBuf` for the found files, or an error if a file system operation fails.
#[instrument]
pub async fn search_sources(
    search_paths: &Vec<PathBuf>,
) -> Result<(Vec<PathBuf>, Vec<PathBuf>), std::io::Error> {
    let mut found: Vec<PathBuf> = vec![];
    let mut configs: Vec<PathBuf> = vec![];

    for dir in search_paths {
        let dir_metadata = fs::metadata(&dir).await;

        if dir_metadata.map_or_else(|_| false, |meta| meta.is_dir()) {
            let mut entries = fs::read_dir(&dir).await?;
            while let Ok(Some(entry)) = entries.next_entry().await {
                let path = entry.path();
                if !path.is_file() {
                    continue;
                }
                if path.ends_with(AXL_VERSION_EXTENSION) {
                    // version.axl files are not evaluated
                } else if path.ends_with(AXL_CONFIG_EXTENSION) {
                    configs.push(path);
                } else if path
                    .extension()
                    .map_or(false, |e| e == AXL_SCRIPT_EXTENSION)
                {
                    found.push(path);
                }
            }
        }
    }

    Ok((found, configs))
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::{TempDir, tempdir};
    use tokio::fs as tokio_fs;

    /// Set up a temp directory with the given markers. Each entry is a
    /// path relative to the temp root; empty content is written. Returns
    /// the temp handle (kept alive for the test) and the root path.
    async fn setup(layout: &[&str]) -> (TempDir, PathBuf) {
        let tmp = tempdir().unwrap();
        let root = tmp.path().to_path_buf();
        for rel in layout {
            let p = root.join(rel);
            if let Some(parent) = p.parent() {
                tokio_fs::create_dir_all(parent).await.unwrap();
            }
            tokio_fs::write(&p, "").await.unwrap();
        }
        (tmp, root)
    }

    /// Aspect marker outside, Bazel marker in a sub-workspace: the two
    /// roots diverge at exactly the boundary their consumers care about.
    #[tokio::test]
    async fn aspect_and_bazel_roots_diverge_in_sub_workspace() {
        let (_tmp, root) = setup(&[".aspect/version.axl", "e2e/MODULE.bazel"]).await;
        let cwd = root.join("e2e/sub");
        tokio_fs::create_dir_all(&cwd).await.unwrap();

        assert_eq!(find_aspect_root(&cwd).await, Some(root.clone()));
        assert_eq!(find_bazel_root(&cwd).await, Some(root.join("e2e")));
    }

    /// Pure-Aspect workspace → both roots resolve to the Aspect marker.
    #[tokio::test]
    async fn bazel_root_falls_back_to_aspect_marker() {
        let (_tmp, root) = setup(&[".aspect/version.axl"]).await;
        let cwd = root.join("sub");
        tokio_fs::create_dir_all(&cwd).await.unwrap();

        assert_eq!(find_aspect_root(&cwd).await, Some(root.clone()));
        assert_eq!(find_bazel_root(&cwd).await, Some(root));
    }

    /// Pure-Bazel monorepo → both roots resolve to the Bazel marker.
    #[tokio::test]
    async fn aspect_root_falls_back_to_bazel_marker() {
        let (_tmp, root) = setup(&["MODULE.bazel"]).await;
        let cwd = root.join("sub");
        tokio_fs::create_dir_all(&cwd).await.unwrap();

        assert_eq!(find_aspect_root(&cwd).await, Some(root.clone()));
        assert_eq!(find_bazel_root(&cwd).await, Some(root));
    }

    /// No markers anywhere → both roots are `None`; callers supply their
    /// own fallback (typically cwd).
    #[tokio::test]
    async fn both_roots_are_none_when_no_markers() {
        let (_tmp, root) = setup(&[]).await;
        let cwd = root.join("sub");
        tokio_fs::create_dir_all(&cwd).await.unwrap();

        assert_eq!(find_aspect_root(&cwd).await, None);
        assert_eq!(find_bazel_root(&cwd).await, None);
    }

    /// `MODULE.aspect` is recognized as an Aspect marker.
    #[tokio::test]
    async fn aspect_root_recognizes_module_aspect() {
        let (_tmp, root) = setup(&["MODULE.aspect"]).await;
        let cwd = root.join("sub");
        tokio_fs::create_dir_all(&cwd).await.unwrap();

        assert_eq!(find_aspect_root(&cwd).await, Some(root.clone()));
        assert_eq!(find_bazel_root(&cwd).await, Some(root));
    }

    /// Every documented Bazel marker is recognized — guards against
    /// silent drift in `BAZEL_BOUNDARY_FILES`.
    #[tokio::test]
    async fn bazel_root_recognizes_every_marker() {
        for marker in project_root::BAZEL_BOUNDARY_FILES {
            let (_tmp, root) = setup(&[marker]).await;
            let cwd = root.join("sub");
            tokio_fs::create_dir_all(&cwd).await.unwrap();
            assert_eq!(find_bazel_root(&cwd).await, Some(root), "marker: {marker}");
        }
    }

    /// `.git` directory found by walking up from a subdirectory.
    #[tokio::test]
    async fn git_root_found_from_subdirectory() {
        let (_tmp, root) = setup(&[]).await;
        tokio_fs::create_dir_all(root.join(".git")).await.unwrap();
        let cwd = root.join("a/b/c");
        tokio_fs::create_dir_all(&cwd).await.unwrap();

        assert_eq!(find_git_root(&cwd).await, Some(root));
    }

    /// `.git` file (git worktree) is recognized.
    #[tokio::test]
    async fn git_root_recognizes_worktree_git_file() {
        let (_tmp, root) = setup(&[".git"]).await;
        let cwd = root.join("sub");
        tokio_fs::create_dir_all(&cwd).await.unwrap();

        assert_eq!(find_git_root(&cwd).await, Some(root));
    }

    /// Returns `None` when not inside any git repository.
    #[tokio::test]
    async fn git_root_none_outside_repo() {
        let (_tmp, root) = setup(&[]).await;
        let cwd = root.join("sub");
        tokio_fs::create_dir_all(&cwd).await.unwrap();

        assert_eq!(find_git_root(&cwd).await, None);
    }

    /// `~/.aspect/config.axl` is found when present.
    #[tokio::test]
    async fn user_config_found_when_present() {
        let (_tmp, home) = setup(&[".aspect/config.axl"]).await;

        assert_eq!(
            find_user_config(Some(&home)).await,
            Some(home.join(".aspect/config.axl"))
        );
    }

    /// Missing file, missing home dir, and a `config.axl` directory all
    /// resolve to no user config.
    #[tokio::test]
    async fn user_config_absent_cases() {
        let (_tmp, home) = setup(&[]).await;
        assert_eq!(find_user_config(Some(&home)).await, None);
        assert_eq!(find_user_config(None).await, None);

        tokio_fs::create_dir_all(home.join(".aspect/config.axl"))
            .await
            .unwrap();
        assert_eq!(find_user_config(Some(&home)).await, None);
    }

    /// When aspect and git roots diverge (e.g. git repo contains a
    /// sub-workspace with its own Bazel markers), git root is the outermost
    /// ancestor with `.git` and is independent of the Bazel/Aspect roots.
    #[tokio::test]
    async fn git_root_is_independent_of_bazel_and_aspect_roots() {
        let (_tmp, root) = setup(&[".git", "e2e/MODULE.bazel"]).await;
        let cwd = root.join("e2e/sub");
        tokio_fs::create_dir_all(&cwd).await.unwrap();

        assert_eq!(find_git_root(&cwd).await, Some(root.clone()));
        assert_eq!(find_bazel_root(&cwd).await, Some(root.join("e2e")));
    }
}
