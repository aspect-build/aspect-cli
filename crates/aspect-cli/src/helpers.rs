use std::path::PathBuf;

use axl_runtime::module::BOUNDARY_FILE as AXL_BOUNDARY_FILE;
use tokio::fs;
use tracing::instrument;

// Constants for special directory names used in module resolution.
// These define the structure for local modules (e.g., .aspect/axl/module_name).
pub const DOT_ASPECT_FOLDER: &str = ".aspect";

pub const AXL_SCRIPT_EXTENSION: &str = "axl";

/// Asynchronously finds the repository root starting from the given `current_work_dir`.
/// It traverses the ancestors of `current_work_dir` from deepest to shallowest.
/// The repository root is identified as the first (deepest) ancestor directory that contains
/// at least one of the following boundary files: AXL_BOUNDARY_FILE, MODULE.bazel,
/// MODULE.bazel.lock, REPO.bazel, WORKSPACE, or WORKSPACE.bazel.
/// If such a directory is found, it returns Ok with the PathBuf of that directory.
/// If no repository root is found among the ancestors, it returns Err(()).
#[instrument]
pub async fn find_repo_root(current_work_dir: &PathBuf) -> Result<PathBuf, ()> {
    // Returns an Err if the path exists
    async fn err_if_exists(path: PathBuf) -> Result<(), ()> {
        match fs::try_exists(path).await {
            Ok(true) => Err(()),
            Ok(false) => Ok(()),
            Err(_) => Ok(()),
        }
    }

    for ancestor in current_work_dir.ancestors().into_iter() {
        let result = tokio::try_join!(
            err_if_exists(ancestor.join(AXL_BOUNDARY_FILE)),
            err_if_exists(ancestor.join("MODULE.bazel")),
            err_if_exists(ancestor.join("MODULE.bazel.lock")),
            err_if_exists(ancestor.join("REPO.bazel")),
            err_if_exists(ancestor.join("WORKSPACE")),
            err_if_exists(ancestor.join("WORKSPACE.bazel")),
        );
        // No error means there was no match for any of the branches.
        if result.is_ok() {
            continue;
        } else {
            return Ok(ancestor.to_path_buf());
        }
    }

    return Err(());
}

/// Returns a list of axl search paths by constructing paths from the `repo_root` up to the `current_dir`,
/// appending ".aspect" to each path. If the relative path from `repo_root` to `current_dir` includes
/// a ".aspect" component, the search stops at the parent directory of that ".aspect", excluding
/// ".aspect" and any subdirectories from the results.
#[instrument]
pub fn get_default_axl_search_paths(
    current_work_dir: &PathBuf,
    repo_dir: &PathBuf,
) -> Vec<PathBuf> {
    if let Ok(rel_path) = current_work_dir.strip_prefix(repo_dir) {
        let mut paths = vec![repo_dir.join(DOT_ASPECT_FOLDER)];
        let mut current = repo_dir.clone();
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
pub async fn find_axl_scripts(search_paths: &Vec<PathBuf>) -> Result<Vec<PathBuf>, std::io::Error> {
    let mut found: Vec<PathBuf> = vec![];

    for dir in search_paths {
        let dir_metadata = fs::metadata(&dir).await;

        if dir_metadata.map_or_else(|_| false, |meta| meta.is_dir()) {
            let mut entries = fs::read_dir(&dir).await?;
            while let Ok(Some(entry)) = entries.next_entry().await {
                let path = entry.path();
                if path.is_file()
                    && path
                        .extension()
                        .map(|e| e == AXL_SCRIPT_EXTENSION)
                        .unwrap_or(false)
                {
                    found.push(path);
                }
            }
        }
    }

    Ok(found)
}
