//! The workspace event log: what bazel's repository rules did, decoded as it
//! is written.

use std::path::{Path, PathBuf};

use crate::stream::Gate;
use crate::stream::file::{self, Reservation};

pub mod feed;

/// The flag that names where Bazel writes the workspace event log.
pub const LOG_FILE_FLAG: &str = "--experimental_workspace_rules_log_file";

/// The workspace log path something already asked Bazel for in `flags`, if
/// any. See [`crate::rc::requested_path`] for how empty and relative values
/// resolve.
pub fn requested_path(flags: &[String], verb: &str, directory: Option<&str>) -> Option<PathBuf> {
    crate::rc::requested_path(flags, LOG_FILE_FLAG, verb, directory)
}

/// Where Bazel writes one build's workspace log.
pub enum LogFile {
    /// A path already on the command line or in an rc file. Whoever asked reads
    /// the file there, so it is tailed in place and left behind, and the flag
    /// is not added again: a literal flag would win and repoint Bazel.
    Requested(PathBuf),
    /// A temp path this build owns: put on the command line, and removed once
    /// read.
    Reserved(Reservation),
}

impl LogFile {
    /// The requested path when there is one, else a fresh reservation.
    pub fn plan(requested: Option<PathBuf>) -> Self {
        match requested {
            Some(path) => Self::Requested(path),
            None => Self::Reserved(Reservation::temp("workspace-event-out")),
        }
    }

    pub fn path(&self) -> &Path {
        match self {
            Self::Requested(path) => path,
            Self::Reserved(reservation) => reservation.path(),
        }
    }

    /// The path to name with [`LOG_FILE_FLAG`], when this build owns it.
    pub fn flag_path(&self) -> Option<&Path> {
        match self {
            Self::Requested(_) => None,
            Self::Reserved(reservation) => Some(reservation.path()),
        }
    }

    pub(crate) fn follow(self, gate: Gate) -> file::File {
        match self {
            Self::Requested(path) => file::File::follow(path, gate),
            Self::Reserved(reservation) => reservation.follow(gate),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_requested_path_is_tailed_in_place_without_a_flag() {
        let flags = vec![format!("{LOG_FILE_FLAG}=/tmp/ws.log")];
        let log = LogFile::plan(requested_path(&flags, "build", None));
        assert_eq!(log.path(), Path::new("/tmp/ws.log"));
        assert_eq!(log.flag_path(), None);
    }

    #[test]
    fn without_a_request_the_build_reserves_and_names_a_path() {
        let log = LogFile::plan(requested_path(&[], "build", None));
        let flag_path = log
            .flag_path()
            .expect("a reserved path goes on the command line");
        assert_eq!(flag_path, log.path());
        assert!(
            flag_path
                .file_name()
                .unwrap()
                .to_string_lossy()
                .starts_with("workspace-event-out-")
        );
    }
}
