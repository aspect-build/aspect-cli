//! Runfiles trees built from the manifest, the way `bazel run` builds them.
//!
//! Bazel writes `<exe>.runfiles_manifest` for every executable whenever
//! `--build_runfile_manifests` is on (the default). Whether it also builds the
//! `<exe>.runfiles` symlink tree depends on `--build_runfile_links`, a
//! configuration option: a task that flips it to get a tree invalidates the
//! whole analysis cache. `bazel run` never flips it. After the build it calls
//! `RunfilesTreeUpdater`, which creates the one tree it needs from the
//! manifest, skipping the work when the tree's `MANIFEST` already matches.
//! This module is that updater.
//!
//! Two modes, from `--enable_runfiles` and the host OS, as
//! `BuildConfigurationValue.getRunfileSymlinksMode` resolves them:
//!
//! - [`SymlinksMode::Create`]: the full tree, one symlink per manifest entry,
//!   an empty file for an entry with no target, `MANIFEST` a copy of the
//!   input manifest, and the workspace subdirectory present.
//! - [`SymlinksMode::Skip`] (Windows by default, or `--noenable_runfiles`):
//!   the "minimal" directory, `MANIFEST` and the workspace subdirectory only.
//!   Bazel's launchers find the manifest next to the binary and resolve
//!   runfiles through it.
//!
//! The up-to-date check is the digest of `MANIFEST` against the input
//! manifest, nothing else: no mtimes, no per-entry stat. A mismatch rebuilds
//! the tree from scratch rather than reconciling in place; it is the same
//! number of filesystem operations and far less code, and it only runs when
//! the target's runfiles actually changed.

use std::fs;
use std::io;
use std::path::{Path, PathBuf};

use sha2::{Digest, Sha256};

pub const INPUT_MANIFEST_EXT: &str = ".runfiles_manifest";
pub const RUNFILES_DIR_EXT: &str = ".runfiles";
const OUTPUT_MANIFEST: &str = "MANIFEST";

/// How runfiles are laid out for a local execution.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SymlinksMode {
    /// Build the full symlink tree.
    Create,
    /// Build only the minimal directory: `MANIFEST` plus the workspace
    /// subdirectory.
    Skip,
}

impl SymlinksMode {
    /// The mode `--enable_runfiles=<value>` resolves to on this host.
    /// `None` is the flag's default, `auto`: on everywhere but Windows.
    pub fn from_enable_runfiles(value: Option<bool>) -> Self {
        match value {
            Some(true) => Self::Create,
            Some(false) => Self::Skip,
            None if cfg!(windows) => Self::Skip,
            None => Self::Create,
        }
    }
}

/// One line of a runfiles manifest.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ManifestEntry {
    /// The runfiles-relative path, e.g. `_main/pkg/file`.
    pub path: String,
    /// The absolute path the entry points at; `None` for an empty file.
    pub target: Option<PathBuf>,
}

/// Parse a runfiles manifest.
///
/// Each line is `<runfiles path> <target>` split at the first space. A line
/// that starts with a space is in Bazel's escaped form, used when either side
/// contains a space, newline or backslash: `\s` is a space, `\n` a newline,
/// `\b` a backslash. An empty target means an empty file.
pub fn parse_manifest(text: &str) -> Vec<ManifestEntry> {
    let mut entries = Vec::new();
    for raw in text.lines() {
        if raw.is_empty() {
            continue;
        }
        let (escaped, line) = match raw.strip_prefix(' ') {
            Some(rest) => (true, rest),
            None => (false, raw),
        };
        let (path, target) = match line.split_once(' ') {
            Some((p, t)) => (p, t),
            None => (line, ""),
        };
        let (path, target) = if escaped {
            (unescape(path), unescape(target))
        } else {
            (path.to_owned(), target.to_owned())
        };
        if path.is_empty() {
            continue;
        }
        entries.push(ManifestEntry {
            path,
            target: (!target.is_empty()).then(|| PathBuf::from(target)),
        });
    }
    entries
}

fn unescape(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut chars = s.chars();
    while let Some(c) = chars.next() {
        if c != '\\' {
            out.push(c);
            continue;
        }
        match chars.next() {
            Some('s') => out.push(' '),
            Some('n') => out.push('\n'),
            Some('b') => out.push('\\'),
            Some(other) => {
                out.push('\\');
                out.push(other);
            }
            None => out.push('\\'),
        }
    }
    out
}

/// The input manifest Bazel wrote next to `executable`, if any.
pub fn input_manifest(executable: &Path) -> Option<PathBuf> {
    let manifest = sibling(executable, INPUT_MANIFEST_EXT);
    manifest.is_file().then_some(manifest)
}

/// The runfiles directory for `executable`: `<exe>.runfiles`, named after
/// the executable file including any extension, as Bazel names it.
pub fn runfiles_dir(executable: &Path) -> PathBuf {
    sibling(executable, RUNFILES_DIR_EXT)
}

fn sibling(executable: &Path, ext: &str) -> PathBuf {
    let mut name = executable
        .file_name()
        .map(|n| n.to_os_string())
        .unwrap_or_default();
    name.push(ext);
    executable.with_file_name(name)
}

/// Make `<exe>.runfiles` match `<exe>.runfiles_manifest` and return it.
///
/// Returns `Ok(None)` when there is no input manifest: the executable has no
/// runfiles support (nothing to build) or manifests are disabled, which the
/// caller refuses up front. A tree whose `MANIFEST` already matches the input
/// manifest, whether Bazel built it or an earlier call did, is left alone.
pub fn materialize(
    executable: &Path,
    workspace_name: &str,
    mode: SymlinksMode,
) -> io::Result<Option<PathBuf>> {
    let Some(manifest) = input_manifest(executable) else {
        return Ok(None);
    };
    let dir = runfiles_dir(executable);
    let text = fs::read_to_string(&manifest)?;
    if up_to_date(&dir, &text) {
        ensure_dir(&dir.join(workspace_name))?;
        return Ok(Some(dir));
    }
    build_tree(&dir, &text, workspace_name, mode)?;
    Ok(Some(dir))
}

fn up_to_date(dir: &Path, input_manifest: &str) -> bool {
    let output = dir.join(OUTPUT_MANIFEST);
    let Ok(meta) = fs::symlink_metadata(&output) else {
        return false;
    };
    // Bazel's own updater links MANIFEST to the input manifest, so a symlink
    // there says nothing about the tree next to it; only a copy is a stamp.
    if !meta.is_file() {
        return false;
    }
    match fs::read(&output) {
        Ok(bytes) => Sha256::digest(&bytes)[..] == Sha256::digest(input_manifest.as_bytes())[..],
        Err(_) => false,
    }
}

fn build_tree(
    dir: &Path,
    input_manifest: &str,
    workspace_name: &str,
    mode: SymlinksMode,
) -> io::Result<()> {
    match fs::remove_dir_all(dir) {
        Ok(()) => {}
        Err(err) if err.kind() == io::ErrorKind::NotFound => {}
        Err(err) => return Err(err),
    }
    fs::create_dir_all(dir)?;
    if mode == SymlinksMode::Create {
        for entry in parse_manifest(input_manifest) {
            if entry.path.starts_with('/') || entry.path.split('/').any(|seg| seg == "..") {
                return Err(io::Error::other(format!(
                    "runfiles manifest entry escapes the tree: {}",
                    entry.path
                )));
            }
            let link = dir.join(&entry.path);
            if let Some(parent) = link.parent() {
                fs::create_dir_all(parent)?;
            }
            match entry.target {
                Some(target) => symlink(&target, &link)?,
                None => {
                    fs::File::create(&link)?;
                }
            }
        }
    }
    // Written last: its presence is the stamp that the tree is complete.
    ensure_dir(&dir.join(workspace_name))?;
    fs::write(dir.join(OUTPUT_MANIFEST), input_manifest)?;
    Ok(())
}

fn ensure_dir(path: &Path) -> io::Result<()> {
    match fs::create_dir_all(path) {
        Ok(()) => Ok(()),
        Err(err) if err.kind() == io::ErrorKind::AlreadyExists => Ok(()),
        Err(err) => Err(err),
    }
}

#[cfg(unix)]
fn symlink(target: &Path, link: &Path) -> io::Result<()> {
    std::os::unix::fs::symlink(target, link)
}

#[cfg(windows)]
fn symlink(target: &Path, link: &Path) -> io::Result<()> {
    if target.is_dir() {
        std::os::windows::fs::symlink_dir(target, link)
    } else {
        std::os::windows::fs::symlink_file(target, link)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entry(path: &str, target: Option<&str>) -> ManifestEntry {
        ManifestEntry {
            path: path.to_owned(),
            target: target.map(PathBuf::from),
        }
    }

    #[test]
    fn parses_plain_lines_and_empty_targets() {
        let text = "_main/a/b /abs/a/b\n_main/empty \n_repo_mapping /abs/repo_mapping\n";
        assert_eq!(
            parse_manifest(text),
            vec![
                entry("_main/a/b", Some("/abs/a/b")),
                entry("_main/empty", None),
                entry("_repo_mapping", Some("/abs/repo_mapping")),
            ]
        );
    }

    #[test]
    fn parses_escaped_lines() {
        let text = " _main/with\\sspace /abs/with\\sspace\\band\\bslash\n";
        assert_eq!(
            parse_manifest(text),
            vec![entry(
                "_main/with space",
                Some("/abs/with space\\and\\slash")
            )]
        );
    }

    #[test]
    fn skips_blank_lines() {
        assert!(parse_manifest("\n\n").is_empty());
    }

    #[test]
    fn mode_follows_enable_runfiles() {
        assert_eq!(
            SymlinksMode::from_enable_runfiles(Some(true)),
            SymlinksMode::Create
        );
        assert_eq!(
            SymlinksMode::from_enable_runfiles(Some(false)),
            SymlinksMode::Skip
        );
        let auto = SymlinksMode::from_enable_runfiles(None);
        if cfg!(windows) {
            assert_eq!(auto, SymlinksMode::Skip);
        } else {
            assert_eq!(auto, SymlinksMode::Create);
        }
    }

    #[test]
    fn names_follow_the_executable_file() {
        let exe = Path::new("/out/bin/pkg/tool.sh");
        assert_eq!(
            runfiles_dir(exe),
            PathBuf::from("/out/bin/pkg/tool.sh.runfiles")
        );
        assert_eq!(
            sibling(exe, INPUT_MANIFEST_EXT),
            PathBuf::from("/out/bin/pkg/tool.sh.runfiles_manifest")
        );
    }

    #[cfg(unix)]
    #[test]
    fn builds_skips_and_rebuilds() {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path();
        let data = root.join("data.txt");
        fs::write(&data, "hello").unwrap();
        let exe = root.join("bin").join("tool");
        fs::create_dir_all(exe.parent().unwrap()).unwrap();
        fs::write(&exe, "#!/bin/sh\n").unwrap();
        let manifest = root.join("bin").join("tool.runfiles_manifest");
        fs::write(
            &manifest,
            format!(
                "_main/bin/tool {}\n_main/data.txt {}\n_main/empty \n",
                exe.display(),
                data.display()
            ),
        )
        .unwrap();

        let dir = materialize(&exe, "_main", SymlinksMode::Create)
            .unwrap()
            .unwrap();
        assert_eq!(dir, root.join("bin").join("tool.runfiles"));
        assert_eq!(
            fs::read_to_string(dir.join("_main/data.txt")).unwrap(),
            "hello"
        );
        assert!(
            fs::symlink_metadata(dir.join("_main/data.txt"))
                .unwrap()
                .is_symlink()
        );
        assert_eq!(fs::metadata(dir.join("_main/empty")).unwrap().len(), 0);
        assert!(dir.join("MANIFEST").is_file());

        // Up to date: a stray file survives because nothing is rebuilt.
        fs::write(dir.join("stray"), "").unwrap();
        materialize(&exe, "_main", SymlinksMode::Create).unwrap();
        assert!(dir.join("stray").exists());

        // Changed manifest: the tree is rebuilt and the stray file is gone.
        fs::write(&manifest, format!("_main/data.txt {}\n", data.display())).unwrap();
        materialize(&exe, "_main", SymlinksMode::Create).unwrap();
        assert!(!dir.join("stray").exists());
        assert!(!dir.join("_main/empty").exists());
        assert!(dir.join("_main/data.txt").exists());

        // Skip mode: minimal directory only.
        fs::write(&manifest, format!("_main/data.txt {}\nx\n", data.display())).unwrap();
        materialize(&exe, "_main", SymlinksMode::Skip).unwrap();
        assert!(!dir.join("_main/data.txt").exists());
        assert!(dir.join("_main").is_dir());
        assert!(dir.join("MANIFEST").is_file());
    }

    #[test]
    fn no_manifest_means_no_tree() {
        let tmp = tempfile::tempdir().unwrap();
        let exe = tmp.path().join("tool");
        fs::write(&exe, "").unwrap();
        assert!(
            materialize(&exe, "_main", SymlinksMode::Create)
                .unwrap()
                .is_none()
        );
    }

    #[test]
    fn rejects_entries_that_escape_the_tree() {
        let tmp = tempfile::tempdir().unwrap();
        let exe = tmp.path().join("tool");
        fs::write(&exe, "").unwrap();
        fs::write(
            tmp.path().join("tool.runfiles_manifest"),
            "../escape /etc/passwd\n",
        )
        .unwrap();
        assert!(materialize(&exe, "_main", SymlinksMode::Create).is_err());
    }
}
