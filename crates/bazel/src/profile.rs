//! Bazel's JSON trace profile, which it writes whole by the time its client
//! exits: the server finishes the file before it answers with the exit code.

use std::io;
use std::path::{Path, PathBuf};

/// The flag that names where Bazel writes the profile.
pub const FLAG: &str = "--profile";

/// The profile's name in a history folder. Bazel compresses a profile whose
/// path ends in `.gz`.
pub const FILE_NAME: &str = "command.profile.gz";

/// The profile path something already asked Bazel for in `flags`, if any. See
/// [`crate::rc::requested_path`] for how empty and relative values resolve.
pub fn requested_path(flags: &[String], verb: &str, directory: Option<&str>) -> Option<PathBuf> {
    crate::rc::requested_path(flags, FLAG, verb, directory)
}

/// Copy the profile Bazel wrote at `from` to `to`, once its client has
/// exited. A build that ended before Bazel wrote one has nothing to copy,
/// which is not an error.
pub fn copy(from: &Path, to: &Path) -> io::Result<()> {
    match std::fs::copy(from, to) {
        Err(err) if err.kind() == io::ErrorKind::NotFound && !from.exists() => Ok(()),
        result => result.map(|_| ()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_profile_is_copied_whole() {
        let dir = tempfile::tempdir().unwrap();
        let (from, to) = (dir.path().join(FILE_NAME), dir.path().join("asked.gz"));
        std::fs::write(&from, b"profile").unwrap();
        copy(&from, &to).unwrap();
        assert_eq!(std::fs::read(&to).unwrap(), b"profile");
    }

    #[test]
    fn no_profile_written_is_nothing_to_copy() {
        let dir = tempfile::tempdir().unwrap();
        let to = dir.path().join("asked.gz");
        copy(&dir.path().join(FILE_NAME), &to).unwrap();
        assert!(!to.exists());
    }

    #[test]
    fn a_copy_that_cannot_be_written_is_an_error() {
        let dir = tempfile::tempdir().unwrap();
        let from = dir.path().join(FILE_NAME);
        std::fs::write(&from, b"profile").unwrap();
        assert!(copy(&from, &dir.path().join("missing").join("asked.gz")).is_err());
    }
}
