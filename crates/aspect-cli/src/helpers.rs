use std::path::{Path, PathBuf};

use axl_runtime::module::{
    AXL_CONFIG_EXTENSION, AXL_MODULE_FILE, AXL_SCRIPT_EXTENSION, AXL_VERSION_EXTENSION,
};
use tokio::fs;
use tracing::instrument;

// Constants for special directory names used in module resolution.
// These define the structure for local modules (e.g., .aspect/axl/module_name).
pub const DOT_ASPECT_FOLDER: &str = ".aspect";

/// Boundary files identifying an Aspect project root. Probed before the Bazel
/// markers so a nested Aspect workspace inside a Bazel monorepo (e.g.
/// `/mono/proj/.aspect/version.axl` under `/mono/MODULE.bazel`) resolves to
/// the Aspect workspace, not the outer Bazel one.
const ASPECT_BOUNDARY_FILES: &[&str] = &[AXL_MODULE_FILE, ".aspect/version.axl"];

/// Bazel repository boundary marker files (see
/// https://bazel.build/external/overview#repository), probed only when no
/// Aspect boundary file is found.
const BAZEL_BOUNDARY_FILES: &[&str] = &[
    "MODULE.bazel",
    "MODULE.bazel.lock",
    "REPO.bazel",
    "WORKSPACE",
    "WORKSPACE.bazel",
];

/// Asynchronously finds the project root directory starting from `current_work_dir`.
///
/// Two-pass walk over the ancestors of `current_work_dir`, deepest to shallowest:
/// 1. Returns the first ancestor containing any [`ASPECT_BOUNDARY_FILES`] entry.
/// 2. If none found, returns the first ancestor containing any [`BAZEL_BOUNDARY_FILES`] entry.
/// 3. If still none found, returns `current_work_dir`.
///
/// The Aspect-first ordering lets a nested Aspect workspace inside a Bazel
/// monorepo opt out of the surrounding Bazel root by dropping a `.aspect/`
/// directory or a `MODULE.aspect` file at its boundary.
#[instrument]
pub async fn find_repo_root(current_work_dir: &PathBuf) -> Result<PathBuf, ()> {
    if let Some(root) = find_ancestor_with_any(current_work_dir, ASPECT_BOUNDARY_FILES).await {
        return Ok(root);
    }
    if let Some(root) = find_ancestor_with_any(current_work_dir, BAZEL_BOUNDARY_FILES).await {
        return Ok(root);
    }
    Ok(current_work_dir.clone())
}

/// Walk ancestors of `start` and return the deepest one containing any of `markers`.
async fn find_ancestor_with_any(start: &Path, markers: &[&str]) -> Option<PathBuf> {
    for ancestor in start.ancestors() {
        for marker in markers {
            if fs::try_exists(ancestor.join(marker)).await.unwrap_or(false) {
                return Some(ancestor.to_path_buf());
            }
        }
    }
    None
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
