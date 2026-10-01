use std::path::{Path, PathBuf};

use axl_runtime::module::{AXL_CONFIG_EXTENSION, AXL_SCRIPT_EXTENSION, AXL_VERSION_EXTENSION};
use axl_runtime::project_root::DOT_ASPECT_FOLDER;
use tokio::fs;
use tracing::instrument;

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
}
