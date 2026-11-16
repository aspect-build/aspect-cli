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

/// Where Bazel writes one build's workspace log, a path this build names
/// either way, so a path the caller asked for is written as a copy.
pub enum LogFile {
    /// A path that stays once read: the build's folder in the task
    /// invocation's history.
    Kept(PathBuf),
    /// A temp path this build owns, removed once read.
    Reserved(Reservation),
}

impl LogFile {
    /// A fresh temp reservation, for when there is nowhere to keep the log.
    pub fn reserve() -> Self {
        Self::Reserved(Reservation::temp("workspace-event-out"))
    }

    pub fn path(&self) -> &Path {
        match self {
            Self::Kept(path) => path,
            Self::Reserved(reservation) => reservation.path(),
        }
    }

    pub(crate) fn follow(self, gate: Gate) -> file::File {
        match self {
            Self::Kept(path) => file::File::follow(path, gate),
            Self::Reserved(reservation) => reservation.follow(gate),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_requested_path_is_read_from_the_flags() {
        let flags = vec![format!("{LOG_FILE_FLAG}=/tmp/ws.log")];
        assert_eq!(
            requested_path(&flags, "build", None),
            Some(PathBuf::from("/tmp/ws.log"))
        );
    }

    #[test]
    fn a_reservation_is_a_fresh_temp_path() {
        let log = LogFile::reserve();
        assert!(
            log.path()
                .file_name()
                .unwrap()
                .to_string_lossy()
                .starts_with("workspace-event-out-")
        );
    }
}
