use std::path::PathBuf;
use std::env::current_dir;

use axl_runtime::module::BOUNDARY_FILE as AXL_BOUNDARY_FILE;
use tracing::instrument;
use tokio::fs;

#[instrument]
pub async fn find_repo_root() -> Result<PathBuf, ()> {
    let current_dir = current_dir().map_err(|_| ())?;

    // Returns an Err if the path exists
    async fn err_if_exists(path: PathBuf) -> Result<(), ()> {
        match fs::try_exists(path).await {
            Ok(true) => Err(()),
            Ok(false) => Ok(()),
            Err(_) => Ok(()),
        }
    }

    for ancestor in current_dir.ancestors().into_iter() {
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
