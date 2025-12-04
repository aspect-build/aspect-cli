use std::path::PathBuf;

use axl_runtime::module::{AXL_CONFIG_EXTENSION, AXL_MODULE_FILE, AXL_SCRIPT_EXTENSION};
use tokio::fs;
use tracing::instrument;

// Constants for special directory names used in module resolution.
// These define the structure for local modules (e.g., .aspect/axl/module_name).
pub const DOT_ASPECT_FOLDER: &str = ".aspect";

/// Asynchronously finds the root directory starting from the given `current_work_dir`.
/// It traverses the ancestors of `current_work_dir` from deepest to shallowest.
/// The root dir is identified as the first (deepest) ancestor directory of the current working
/// directory that contains at least one of the following boundary files: MODULE.aspect, MODULE.bazel,
/// MODULE.bazel.lock, REPO.bazel, WORKSPACE, or WORKSPACE.bazel.
/// If such a directory is found, it returns Ok with the PathBuf of that directory.
/// If no such directory is found, returns the `current_work_dir`
#[instrument]
pub async fn find_root_dir(current_work_dir: &PathBuf) -> Result<PathBuf, ()> {
    async fn err_if_exists(path: PathBuf) -> Result<(), ()> {
        match fs::try_exists(path).await {
            Ok(true) => Err(()),
            Ok(false) => Ok(()),
            Err(_) => Ok(()),
        }
    }

    for ancestor in current_work_dir.ancestors().into_iter() {
        let repo_root = tokio::try_join!(
            err_if_exists(ancestor.join(AXL_MODULE_FILE)),
            // Repository boundary marker files: https://bazel.build/external/overview#repository
            err_if_exists(ancestor.join("MODULE.bazel")),
            err_if_exists(ancestor.join("MODULE.bazel.lock")),
            err_if_exists(ancestor.join("REPO.bazel")),
            err_if_exists(ancestor.join("WORKSPACE")),
            err_if_exists(ancestor.join("WORKSPACE.bazel")),
        );
        // No error means there was no match for any of the branches.
        if repo_root.is_ok() {
            continue;
        } else {
            return Ok(ancestor.to_path_buf());
        }
    }

    return Ok(current_work_dir.clone());
}

/// Returns a list of axl search paths by constructing paths from the `root_dir` up to the `current_dir`,
/// appending ".aspect" to each path. If the relative path from `root_dir` to `current_dir` includes
/// a ".aspect" component, the search stops at the parent directory of that ".aspect", excluding
/// ".aspect" and any subdirectories from the results.
#[instrument]
pub fn get_default_axl_search_paths(
    current_work_dir: &PathBuf,
    root_dir: &PathBuf,
) -> Vec<PathBuf> {
    if let Ok(rel_path) = current_work_dir.strip_prefix(root_dir) {
        let mut paths = vec![root_dir.join(DOT_ASPECT_FOLDER)];
        let mut current = root_dir.clone();
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
                if path.ends_with(AXL_CONFIG_EXTENSION) {
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
