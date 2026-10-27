//! The execution log: bazel's record of every spawn, decoded as it is written.

use std::io;
use std::path::{Path, PathBuf};

pub mod feed;
pub mod sink;
pub mod subscription;

/// The flag that names where Bazel writes the compact execution log.
pub const COMPACT_FILE_FLAG: &str = "--execution_log_compact_file";

/// The compact log path something already asked Bazel for in `flags`, if any.
///
/// A deployment wired to an Aspect BES backend, a Workflows runner, or a
/// `.bazelrc` already asks Bazel for the compact log, and whoever asked reads
/// the file at that path. Adding a second flag would win — a literal flag
/// outranks rc content — and silently repoint Bazel away from it.
///
/// An empty value clears the flag rather than naming a file, so it is not a
/// request; treating it as a path would have the reader tail "" and report a
/// clean empty stream. A relative value is joined to `directory`, the spawn's
/// directory, because that is what Bazel resolves it against while the reader
/// runs from the CLI's own cwd.
pub fn requested_path(flags: &[String], verb: &str, directory: Option<&str>) -> Option<PathBuf> {
    crate::rc::last_flag_value(flags, COMPACT_FILE_FLAG, verb)
        .filter(|path| !path.is_empty())
        .map(|path| match directory {
            Some(dir) if Path::new(&path).is_relative() => Path::new(dir).join(path),
            _ => PathBuf::from(path),
        })
}

/// Who writes the compact log, and which sinks still need their own copy.
#[derive(Debug, PartialEq, Eq)]
pub struct Plan {
    /// Where Bazel writes the log, and the reader tails it.
    pub path: PathBuf,
    /// Whether this call owns the flag: nothing asked for the log yet, so the
    /// command line must name `path`.
    pub add_flag: bool,
    /// Compact sinks served by a tee of the stream rather than by Bazel
    /// writing to them directly.
    pub tee: Vec<String>,
}

/// Decide who writes the log. `requested` is [`requested_path`]; `compact` are
/// the paths of the caller's compact-file sinks.
///
/// When something already named the path, tail that file and leave the flag
/// alone. Otherwise this call owns the flag, and a compact sink lends its path,
/// letting Bazel write straight to the caller's destination with no temp file
/// or tee step for that copy.
///
/// Every remaining compact sink is served by the tee, so each still gets its
/// copy — except one naming the very file Bazel is writing, whose
/// `File::create` would truncate it mid-build. Both cases can hold one: a
/// sink's path can already be on the command line, and two sinks can name one
/// path, in which case the first lent it to Bazel.
pub fn plan(requested: Option<PathBuf>, mut compact: Vec<String>) -> Plan {
    let (path, add_flag) = match requested {
        Some(path) => (path, false),
        None => {
            let direct = if compact.is_empty() {
                None
            } else {
                Some(PathBuf::from(compact.remove(0)))
            };
            (feed::ExecLogStream::reserve_path(direct), true)
        }
    };
    compact.retain(|sink| Path::new(sink) != path);
    Plan {
        path,
        add_flag,
        tee: compact,
    }
}

/// Remove a log left at `path` by an *earlier* run, before Bazel starts.
///
/// [`crate::stream::file::File`] opens whatever is at the path as soon as there
/// is something. Left in place the reader opens the stale file immediately,
/// races Bazel's truncation of it and loses: it delivers the previous build's
/// entries — spawns that never ran, inputs that no longer exist — and misses
/// this build's.
///
/// The exposure is in reused paths: a compact sink's path and one already on
/// the command line are both fixed per build, so on a Workflows runner or an
/// Aspect-wired deployment every build finds the last one's log waiting.
/// Removing it is what makes the existence poll mean "wait for *this* build's
/// log"; Bazel creates the file itself, so there is nothing to put back.
///
/// A log that is not there is fine. Anything else is returned for the caller
/// to say.
pub fn clear_stale(path: &Path) -> Result<(), io::Error> {
    match std::fs::remove_file(path) {
        Err(err) if err.kind() != io::ErrorKind::NotFound => Err(err),
        _ => Ok(()),
    }
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
    fn an_empty_value_clears_the_flag_and_is_not_a_request() {
        let f = flags(&["--execution_log_compact_file="]);
        assert_eq!(requested_path(&f, "build", None), None);
    }

    #[test]
    fn a_relative_path_is_joined_to_the_spawn_directory() {
        let f = flags(&["--execution_log_compact_file=out/log.zst"]);
        assert_eq!(
            requested_path(&f, "build", Some("/ws")),
            Some(PathBuf::from("/ws/out/log.zst"))
        );
        assert_eq!(
            requested_path(&f, "build", None),
            Some(PathBuf::from("out/log.zst"))
        );
        let abs = flags(&["--execution_log_compact_file=/abs/log.zst"]);
        assert_eq!(
            requested_path(&abs, "build", Some("/ws")),
            Some(PathBuf::from("/abs/log.zst"))
        );
    }

    #[test]
    fn a_requested_path_leaves_the_flag_alone_and_tees_every_sink_but_its_own() {
        let p = plan(
            Some(PathBuf::from("/tmp/asked.zst")),
            vec!["/tmp/asked.zst".to_string(), "/tmp/copy.zst".to_string()],
        );
        assert_eq!(
            p,
            Plan {
                path: PathBuf::from("/tmp/asked.zst"),
                add_flag: false,
                tee: vec!["/tmp/copy.zst".to_string()],
            }
        );
    }

    #[test]
    fn with_nothing_requested_the_first_compact_sink_lends_its_path() {
        let p = plan(
            None,
            vec![
                "/tmp/first.zst".to_string(),
                "/tmp/first.zst".to_string(),
                "/tmp/second.zst".to_string(),
            ],
        );
        assert!(p.add_flag, "this call owns the flag");
        assert_eq!(p.path, PathBuf::from("/tmp/first.zst"));
        // The duplicate of the lent path would truncate it; the other is teed.
        assert_eq!(p.tee, vec!["/tmp/second.zst".to_string()]);
    }

    #[test]
    fn clearing_a_stale_log_tolerates_its_absence_and_removes_a_present_one() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("log.zst");
        assert!(clear_stale(&path).is_ok(), "nothing to clear is fine");
        std::fs::write(&path, b"old").unwrap();
        assert!(clear_stale(&path).is_ok());
        assert!(!path.exists());
    }
}
