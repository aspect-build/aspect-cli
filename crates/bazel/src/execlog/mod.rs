//! The execution log: bazel's record of every spawn, decoded as it is written.

use std::collections::HashSet;
use std::path::{Path, PathBuf};

pub mod expanded;
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
/// path is written as one of the [`copies`]. `%workspace%` stands for
/// `workspace`, as Bazel expands it in this flag, and a boolean value names
/// no file, as does the `--no` form Bazel 10 takes. The option's old name,
/// which Bazel still takes, is read as it. An empty value names no file, and
/// a relative one is placed against `directory` (see [`place`]).
pub fn requested_path(
    flags: &[String],
    verb: &str,
    directory: Option<&str>,
    workspace: Option<&Path>,
) -> Option<PathBuf> {
    let flags = with_current_name(flags);
    // Bazel 10 reads the option as a boolean too, so its `--no` form turns
    // off a path set earlier.
    let off = crate::rc::last_word(&flags, verb, |token| {
        crate::rc::bool_token(token, COMPACT_FILE_FLAG).map(|word| word == Ok(false))
    });
    if off == Some(true) {
        return None;
    }
    let value = crate::rc::last_flag_value(&flags, COMPACT_FILE_FLAG, verb)?;
    // Bazel 10 reads a boolean here (off, or stream it to remote storage),
    // which names no file.
    if value.is_empty() || crate::rc::parse_bool(&value).is_some() {
        return None;
    }
    Some(place(&value, directory, workspace))
}

/// The file an execution log flag's `value` names, as Bazel places all three
/// formats: `~/` is the user's home, `%workspace%` the `workspace` root, and
/// a relative path is under `directory`, the spawn's working directory.
fn place(value: &str, directory: Option<&str>, workspace: Option<&Path>) -> PathBuf {
    let path = crate::rc::home_path(value);
    let text = path.to_string_lossy();
    let path = match workspace {
        Some(root) if text.contains("%workspace%") => {
            PathBuf::from(text.replace("%workspace%", &root.to_string_lossy()))
        }
        _ => path,
    };
    match directory {
        Some(dir) if path.is_relative() => Path::new(dir).join(path),
        _ => path,
    }
}

/// The name Bazel 7.1 to 7.3 knew the compact log by, which later ones still
/// take as [`COMPACT_FILE_FLAG`].
const OLD_COMPACT_FILE_FLAG: &str = "--experimental_execution_log_compact_file";

/// Whether `version` knows [`COMPACT_FILE_FLAG`] by that name: 7.4 and later,
/// or a version that cannot be told. As the rest of the CLI gates by version,
/// a prerelease of 7.4 knows it.
pub fn knows_compact_file_flag(version: Option<&semver::Version>) -> bool {
    crate::rc::constraint_matches(">=7.4.0", version).unwrap_or(true)
}

/// The compact log's flag as `version` knows it: [`COMPACT_FILE_FLAG`] from
/// 7.4, or for a version that cannot be told, and the old name before. The
/// two name one option, so either outranks a caller's value under the other.
pub fn compact_file_flag(version: Option<&semver::Version>) -> &'static str {
    if knows_compact_file_flag(version) {
        COMPACT_FILE_FLAG
    } else {
        OLD_COMPACT_FILE_FLAG
    }
}

/// `flags` with the compact log's old name spelled as its current one, on the
/// command line and in `--default_override` payloads alike.
fn with_current_name(flags: &[String]) -> Vec<String> {
    let negated = |name: &str| format!("--no{}", &name[2..]);
    let renames = [
        (negated(OLD_COMPACT_FILE_FLAG), negated(COMPACT_FILE_FLAG)),
        (
            OLD_COMPACT_FILE_FLAG.to_owned(),
            COMPACT_FILE_FLAG.to_owned(),
        ),
    ];
    flags
        .iter()
        .map(|flag| {
            // The payload is a suffix of the token, so the wrapper is kept.
            let option = crate::rc::override_payload(flag).unwrap_or(flag);
            let head = &flag[..flag.len() - option.len()];
            for (old, new) in &renames {
                if let Some(rest) = option.strip_prefix(old.as_str())
                    && (rest.is_empty() || rest.starts_with('='))
                {
                    return format!("{head}{new}{rest}");
                }
            }
            flag.clone()
        })
        .collect()
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
            requested_path(&f, "build", None, None),
            Some(PathBuf::from("/tmp/log.zst"))
        );
        assert_eq!(requested_path(&flags(&[]), "build", None, None), None);
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
        let flags = flags(&["--execution_log_compact_file=%workspace%/exec.log"]);
        assert_eq!(
            requested_path(&flags, "build", Some("/ws/pkg"), Some(Path::new("/ws"))),
            Some(PathBuf::from("/ws/exec.log"))
        );
    }

    /// As `aspect setup bazelrc` writes it into a checkout's rc, which a
    /// rendered rc passes on as an override.
    #[test]
    fn workspace_in_an_rc_path_is_the_workspace_root() {
        let flags = flags(&[
            "--default_override=0:common=--execution_log_compact_file=%workspace%/.aspect/generated/exec.log.zstd",
        ]);
        assert_eq!(
            requested_path(&flags, "test", Some("/ws"), Some(Path::new("/ws"))),
            Some(PathBuf::from("/ws/.aspect/generated/exec.log.zstd"))
        );
    }

    #[test]
    fn the_no_form_turns_off_a_path_set_before_it() {
        for line in [
            vec![
                "--execution_log_compact_file=/x",
                "--noexecution_log_compact_file",
            ],
            vec![
                "--default_override=0:build=--execution_log_compact_file=/x",
                "--noexperimental_execution_log_compact_file",
            ],
            vec![
                "--default_override=0:build=--execution_log_compact_file=/x",
                "--default_override=1:build=--noexperimental_execution_log_compact_file",
            ],
        ] {
            assert_eq!(
                requested_path(&flags(&line), "build", None, None),
                None,
                "{line:?}"
            );
        }
        let on_again = flags(&[
            "--noexecution_log_compact_file",
            "--execution_log_compact_file=/y",
        ]);
        assert_eq!(
            requested_path(&on_again, "build", None, None),
            Some(PathBuf::from("/y"))
        );
    }

    #[test]
    fn the_flag_is_spelled_as_the_version_knows_it() {
        let v = |s: &str| semver::Version::parse(s).unwrap();
        assert_eq!(compact_file_flag(Some(&v("7.3.2"))), OLD_COMPACT_FILE_FLAG);
        assert_eq!(compact_file_flag(Some(&v("7.4.0"))), COMPACT_FILE_FLAG);
        assert_eq!(compact_file_flag(Some(&v("8.1.0"))), COMPACT_FILE_FLAG);
        assert_eq!(compact_file_flag(None), COMPACT_FILE_FLAG);
    }

    #[test]
    fn the_old_name_is_read_as_the_current_one() {
        let line = flags(&["--experimental_execution_log_compact_file=/tmp/old.zst"]);
        assert_eq!(
            requested_path(&line, "build", None, None),
            Some(PathBuf::from("/tmp/old.zst"))
        );
        let rc = flags(&[
            "--default_override=0:common=--experimental_execution_log_compact_file=/tmp/rc.zst",
        ]);
        assert_eq!(
            requested_path(&rc, "build", None, None),
            Some(PathBuf::from("/tmp/rc.zst"))
        );
        let both = flags(&[
            "--execution_log_compact_file=/tmp/new.zst",
            "--experimental_execution_log_compact_file=/tmp/old.zst",
        ]);
        assert_eq!(
            requested_path(&both, "build", None, None),
            Some(PathBuf::from("/tmp/old.zst"))
        );
    }

    #[test]
    fn a_boolean_names_no_file() {
        for value in ["true", "false", "0"] {
            let flags = flags(&[&format!("--execution_log_compact_file={value}")]);
            assert_eq!(requested_path(&flags, "build", None, None), None, "{value}");
        }
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
