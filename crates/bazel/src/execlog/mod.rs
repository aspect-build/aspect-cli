//! The execution log: bazel's record of every spawn, decoded as it is written.

use std::collections::HashSet;
use std::path::{Path, PathBuf};

pub mod feed;
pub mod sink;
mod spawn_metrics;
pub mod telemetry;

/// The log's name in a history folder.
pub const FILE_NAME: &str = "execlog.zstd";

/// The flag that names where Bazel writes the compact execution log.
pub const COMPACT_FILE_FLAG: &str = "--execution_log_compact_file";

/// The `--execution_log_compact_file` value that asks for the log in the task
/// invocation's history and nowhere else. A feature injects it as an ordinary
/// flag, so the user's flag filters and the version gate apply to it like any
/// other; [`take_history_request`] takes it off the command line.
pub const IN_HISTORY: &str = "%aspect_history%";

/// `flags` without any request for [`IN_HISTORY`], and whether there was one:
/// a literal flag in either form, or an rc entry the CLI restated as a
/// `--default_override` token (a `common`-scoped injection renders as one).
pub fn take_history_request(flags: Vec<String>) -> (Vec<String>, bool) {
    let inline = format!("{COMPACT_FILE_FLAG}={IN_HISTORY}");
    let mut asked = false;
    let mut kept = Vec::with_capacity(flags.len());
    let mut flags = flags.into_iter().peekable();
    while let Some(flag) = flags.next() {
        if flag == inline || crate::rc::override_payload(&flag) == Some(inline.as_str()) {
            asked = true;
        } else if flag == COMPACT_FILE_FLAG && flags.peek().is_some_and(|v| v == IN_HISTORY) {
            flags.next();
            asked = true;
        } else {
            kept.push(flag);
        }
    }
    (kept, asked)
}

/// The compact log path something already asked Bazel for in `flags`, if any.
///
/// A deployment wired to an Aspect BES backend, a Workflows runner, or a
/// `.bazelrc` can ask Bazel for the compact log, and whoever asked reads the
/// file at that path. When the CLI streams the log it names a file of its own,
/// which outranks that one (the option is single-valued), so the asked-for
/// path is written as one of the [`copies`]. `%workspace%` stands for the
/// workspace root, as Bazel expands it in this flag; see
/// [`crate::rc::requested_path`] for how empty and relative values resolve.
pub fn requested_path(flags: &[String], verb: &str, directory: Option<&str>) -> Option<PathBuf> {
    let path = crate::rc::requested_path(flags, COMPACT_FILE_FLAG, verb, None)?;
    let text = path.to_string_lossy();
    let path = match crate::rc::workspace_root(directory) {
        Some(root) if text.contains("%workspace%") => {
            PathBuf::from(text.replace("%workspace%", &root.to_string_lossy()))
        }
        _ => path,
    };
    Some(match directory {
        Some(dir) if path.is_relative() => Path::new(dir).join(path),
        _ => path,
    })
}

/// The compact copies the CLI writes from the stream: the path the caller
/// asked Bazel for, if any (see [`requested_path`]), then each compact sink.
///
/// Bazel writes only a file the CLI reserved, so every copy comes from the
/// stream, whole and in order. Paths naming one file share one copy; they are
/// compared as written (`a//b` matches `a/b`, but `./x` does not match `x` and
/// symlinks are not resolved).
pub fn copies(requested: Option<PathBuf>, compact: Vec<String>) -> Vec<String> {
    let mut seen = HashSet::new();
    requested
        .map(|path| path.to_string_lossy().into_owned())
        .into_iter()
        .chain(compact)
        .filter(|path| seen.insert(PathBuf::from(path)))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn flags(items: &[&str]) -> Vec<String> {
        items.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn a_requested_path_is_read_from_the_flag() {
        let f = flags(&["--execution_log_compact_file=/tmp/log.zst"]);
        assert_eq!(
            requested_path(&f, "build", None),
            Some(PathBuf::from("/tmp/log.zst"))
        );
        assert_eq!(requested_path(&flags(&[]), "build", None), None);
    }

    #[test]
    fn the_requested_path_is_the_first_copy() {
        let c = copies(
            Some(PathBuf::from("/tmp/asked.zst")),
            vec!["/tmp/copy.zst".to_string()],
        );
        assert_eq!(c, vec!["/tmp/asked.zst", "/tmp/copy.zst"]);
    }

    #[test]
    fn paths_naming_one_file_share_a_copy() {
        let c = copies(
            Some(PathBuf::from("/tmp/asked.zst")),
            vec![
                "/tmp/asked.zst".to_string(),
                "/tmp/copy.zst".to_string(),
                "/tmp//copy.zst".to_string(),
            ],
        );
        assert_eq!(c, vec!["/tmp/asked.zst", "/tmp/copy.zst"]);
    }

    /// `aspect_flags.axl` spells this value out; its test pins the same one.
    #[test]
    fn the_history_request_value_is_the_one_aspect_flags_injects() {
        assert_eq!(IN_HISTORY, "%aspect_history%");
    }

    #[test]
    fn workspace_in_a_requested_path_is_the_workspace_root() {
        let ws = tempfile::tempdir().unwrap();
        std::fs::write(ws.path().join("MODULE.bazel"), "").unwrap();
        let pkg = ws.path().join("pkg");
        std::fs::create_dir(&pkg).unwrap();
        let flags = flags(&["--execution_log_compact_file=%workspace%/exec.log"]);
        assert_eq!(
            requested_path(&flags, "build", pkg.to_str()),
            Some(ws.path().join("exec.log"))
        );
    }

    #[test]
    fn a_history_request_is_taken_off_the_command_line() {
        let (kept, asked) = take_history_request(flags(&[
            "--a",
            "--execution_log_compact_file=%aspect_history%",
            "--execution_log_compact_file",
            "%aspect_history%",
            "--execution_log_compact_file=/tmp/mine.zst",
        ]));
        assert!(asked);
        assert_eq!(
            kept,
            flags(&["--a", "--execution_log_compact_file=/tmp/mine.zst"])
        );
        let (kept, asked) = take_history_request(flags(&[
            "--default_override=0:common=--execution_log_compact_file=%aspect_history%",
            "--default_override=0:build=--a",
        ]));
        assert!(asked, "an rc entry restated as an override");
        assert_eq!(kept, flags(&["--default_override=0:build=--a"]));
        let (kept, asked) = take_history_request(flags(&["--a"]));
        assert!(!asked);
        assert_eq!(kept, flags(&["--a"]));
    }

    #[test]
    fn with_nothing_asked_for_there_are_no_copies() {
        assert!(copies(None, vec![]).is_empty());
    }
}
