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

/// The profile path something already asked Bazel for in `flags`, if any. See
/// [`crate::rc::requested_path`] for how empty and relative values resolve.
pub fn requested_path(flags: &[String], verb: &str, directory: Option<&str>) -> Option<PathBuf> {
    crate::rc::requested_path(flags, FLAG, verb, directory)
}

/// The option that turns the profile on or off.
const GENERATE: &str = "--generate_json_trace_profile";

/// Whether `flags` turn the profile off for `verb`. A `--profile` would turn
/// it back on, so a caller who turned it off is not given one. Read as
/// [`crate::rc::last_flag_value`] reads a value: a literal beats an rc entry,
/// and the last of each wins.
pub fn turned_off(flags: &[String], verb: &str) -> bool {
    let negated = format!("--no{}", &GENERATE[2..]);
    let value = |token: &str| -> Option<bool> {
        if token == GENERATE {
            Some(true)
        } else if token == negated {
            Some(false)
        } else {
            let value = token.strip_prefix(GENERATE)?.strip_prefix('=')?;
            Some(!matches!(value, "false" | "no" | "0"))
        }
    };
    let last = |tokens: &[&str]| tokens.iter().rev().find_map(|t| value(t));
    let tokens: Vec<&str> = flags.iter().map(String::as_str).collect();
    let literals: Vec<&str> = tokens
        .iter()
        .copied()
        .filter(|t| !crate::rc::is_default_override(t))
        .collect();
    let on = last(&literals).or_else(|| {
        crate::rc::override_buckets(&tokens, verb)
            .iter()
            .rev()
            .find_map(|bucket| last(bucket))
    });
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
