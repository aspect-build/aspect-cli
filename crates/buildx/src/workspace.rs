use crate::engine::{Engine, EngineOptions};
use crate::output_base;
use anyhow::{Context, Result, bail};
use blazon::release::Release;
use std::path::{Path, PathBuf};

/// Nearest ancestor of `from` holding a `MODULE.bazel` or `WORKSPACE`.
pub fn find(from: &Path) -> Result<PathBuf> {
    let mut dir = Some(from);
    while let Some(d) = dir {
        if d.join("MODULE.bazel").is_file()
            || d.join("WORKSPACE").is_file()
            || d.join("WORKSPACE.bazel").is_file()
        {
            return Ok(d.to_path_buf());
        }
        dir = d.parent();
    }
    bail!("no Bazel workspace above {}", from.display())
}

/// The version in the workspace's `.bazelversion`; there is no fallback.
pub fn bazel_version(workspace: &Path) -> Result<String> {
    let path = workspace.join(".bazelversion");
    let text = std::fs::read_to_string(&path)
        .with_context(|| format!("{} is required", path.display()))?;
    let version = text
        .lines()
        .map(str::trim)
        .find(|l| !l.is_empty() && !l.starts_with('#'))
        .unwrap_or("");
    if version.is_empty() {
        bail!("{} names no version", path.display());
    }
    Ok(version.to_string())
}

/// Starts the engine for the workspace in Bazel's own output base, fetching
/// the release on first use.
pub fn start(workspace: &Path) -> Result<Engine> {
    let version = bazel_version(workspace)?;
    let install = Release { version }.install(None)?;
    let output_user_root = output_base::output_user_root()?;
    let output_base = output_base::output_base(&output_user_root, workspace);
    let cwd = std::env::current_dir()?.canonicalize()?;
    let relative_working_directory = cwd
        .strip_prefix(workspace)
        .map(|p| p.to_string_lossy().into_owned())
        .unwrap_or_default();
    Engine::start(EngineOptions {
        install,
        workspace: workspace.to_path_buf(),
        output_user_root,
        output_base,
        heap: "2g".into(),
        relative_working_directory,
    })
}
