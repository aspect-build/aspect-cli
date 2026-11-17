//! Bazel's JSON trace profile, which it writes whole by the time its client
//! exits: the server finishes the file before it answers with the exit code.

use std::io;
use std::path::{Path, PathBuf};

/// The flag that names where Bazel writes the profile.
pub const FLAG: &str = "--profile";

/// The profile's name in a history folder. Bazel compresses a profile whose
/// path ends in `.gz`, and writes plain JSON otherwise.
pub const FILE_NAME: &str = "command.profile.gz";

/// [`FILE_NAME`], uncompressed, for a caller who asked for a path that does
/// not end in `.gz`.
pub const JSON_FILE_NAME: &str = "command.profile.json";

/// The history name for a profile whose copy goes to `requested`, so the copy
/// is in the format its own name tells Bazel and its readers to expect.
pub fn file_name(requested: Option<&Path>) -> &'static str {
    match requested {
        Some(path) if !path.to_string_lossy().ends_with(".gz") => JSON_FILE_NAME,
        _ => FILE_NAME,
    }
}

/// The profile path something already asked Bazel for in `flags`, if any. A
/// relative one is under `workspace`, the root Bazel resolves it against, not
/// the directory the command runs in; `None` for one when there is no
/// workspace. See [`crate::rc::requested_path`] for empty and `~/` values.
pub fn requested_path(flags: &[String], verb: &str, workspace: Option<&Path>) -> Option<PathBuf> {
    let path = crate::rc::requested_home_path(flags, FLAG, verb, None)?;
    if path.is_absolute() {
        return Some(path);
    }
    // Bazel runs no command outside a workspace, so a root not found here is
    // one this cannot name the profile's path in; the profile is left to Bazel.
    workspace.map(|root| root.join(path))
}

/// The option that turns the profile on or off, under its name and the old
/// one Bazel still takes.
const GENERATE: [&str; 2] = [
    "--generate_json_trace_profile",
    "--experimental_generate_json_trace_profile",
];

/// Whether `flags` turn the profile off for `verb`, so a caller who turned it
/// off is not given a `--profile`. Read as [`crate::rc::last_flag_value`]
/// reads a value: a literal beats an rc entry, and the last of each wins.
pub fn turned_off(flags: &[String], verb: &str) -> bool {
    let value = |token: &str| -> Option<bool> {
        let name = GENERATE.iter().find(|name| {
            token
                .strip_prefix(**name)
                .is_some_and(|rest| rest.is_empty() || rest.starts_with('='))
                || token.strip_prefix("--no") == Some(&name[2..])
        })?;
        if token == *name {
            Some(true)
        } else if token.starts_with("--no") {
            Some(false)
        } else {
            let value = token.strip_prefix(*name)?.strip_prefix('=')?;
            Some(!matches!(
                value.to_ascii_lowercase().as_str(),
                "false" | "f" | "no" | "n" | "0"
            ))
        }
    };
    let on = crate::rc::last_word(flags, verb, value);
    on == Some(false)
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

    fn strings(flags: &[&str]) -> Vec<String> {
        flags.iter().map(|f| f.to_string()).collect()
    }

    #[test]
    fn the_profile_is_off_only_when_the_last_word_says_so() {
        assert!(!turned_off(&[], "build"));
        assert!(turned_off(
            &strings(&["--nogenerate_json_trace_profile"]),
            "build"
        ));
        assert!(turned_off(
            &strings(&["--generate_json_trace_profile=false"]),
            "build"
        ));
        assert!(turned_off(
            &strings(&["--generate_json_trace_profile=F"]),
            "build"
        ));
        assert!(turned_off(
            &strings(&["--noexperimental_generate_json_trace_profile"]),
            "build"
        ));
        assert!(!turned_off(
            &strings(&[
                "--nogenerate_json_trace_profile",
                "--generate_json_trace_profile"
            ]),
            "build"
        ));
        assert!(turned_off(
            &strings(&["--default_override=0:build=--nogenerate_json_trace_profile"]),
            "build"
        ));
        assert!(
            !turned_off(
                &strings(&[
                    "--default_override=0:build=--nogenerate_json_trace_profile",
                    "--generate_json_trace_profile",
                ]),
                "build"
            ),
            "a literal beats an rc entry"
        );
    }

    #[test]
    fn a_relative_profile_is_under_the_workspace_root() {
        let ws = Some(Path::new("/ws"));
        assert_eq!(
            requested_path(&strings(&["--profile=p.json"]), "build", ws),
            Some(PathBuf::from("/ws/p.json"))
        );
        assert_eq!(
            requested_path(&strings(&["--profile=/abs/p.gz"]), "build", ws),
            Some(PathBuf::from("/abs/p.gz"))
        );
        assert_eq!(
            requested_path(&strings(&["--profile=p.json"]), "build", None),
            None,
            "outside a workspace a relative profile is left to Bazel"
        );
    }

    #[test]
    fn a_requested_path_names_the_format() {
        assert_eq!(file_name(None), FILE_NAME);
        assert_eq!(file_name(Some(Path::new("/tmp/p.gz"))), FILE_NAME);
        assert_eq!(file_name(Some(Path::new("/tmp/p.json"))), JSON_FILE_NAME);
        assert_eq!(file_name(Some(Path::new("/tmp/p"))), JSON_FILE_NAME);
    }

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
