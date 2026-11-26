//! The raw log: every byte the client wrote to the capture (its
//! stderr, and its stdout when the two are one file), before any stage changed
//! it. A pipeline that can drop or rewrite a line keeps one, so nothing it
//! hides is lost; the summary at the end of the console names the file, and
//! `build.output_log` always does.
//!
//! A log goes where its [`Place`] says: a file of its own (`console.log` in a
//! Bazel invocation's history folder), or a fresh name in a directory shared
//! with others, `console-<millis>-<pid>-<n>.log`, so names sort oldest first.
//! The files of what a `collapse_similar` stage folded go beside it
//! ([`RawLog::sibling`]). Nothing here removes them, and nothing caps their
//! size: how long they are kept is the caller's to decide. The raw log
//! holds whatever a stage redacted, so each file is `0600`, and every
//! directory created for them `0700`; an existing one keeps its mode.

use std::fs::{self, File, OpenOptions};
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU32, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

/// The raw log's name where it has a folder of its own (a Bazel invocation's
/// in the history).
pub const FILE_NAME: &str = "console.log";

const PREFIX: &str = "console-";
const SUFFIX: &str = ".log";

/// Why a [`RawLog`] stopped short of the console.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub enum Cut {
    None = 0,
    /// A write to it failed (a full disk, say).
    Failed = 1,
}

impl Cut {
    pub fn from_code(code: u8) -> Cut {
        match code {
            1 => Cut::Failed,
            _ => Cut::None,
        }
    }
}

/// Where a [`RawLog`] goes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Place {
    /// This file, which must not exist yet: a Bazel invocation's own.
    At(PathBuf),
    /// A fresh name in this directory, shared with other logs: for a
    /// pipeline run on its own, as tests do.
    In(PathBuf),
}

impl std::fmt::Display for Place {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Place::At(path) => write!(f, "{}", path.display()),
            Place::In(dir) => write!(f, "a new file in {}", dir.display()),
        }
    }
}

/// One invocation's raw log, open for writing.
#[derive(Debug)]
pub struct RawLog {
    path: PathBuf,
    file: Option<File>,
    /// Why the log stopped short of the console, if it did: see [`Cut`].
    cut: std::sync::Arc<std::sync::atomic::AtomicU8>,
}

impl RawLog {
    /// Create a new log where `place` says.
    pub fn create(place: &Place) -> io::Result<RawLog> {
        let path = match place {
            Place::At(path) => path.clone(),
            Place::In(dir) => {
                static SEQ: AtomicU32 = AtomicU32::new(0);
                let millis = SystemTime::now()
                    .duration_since(UNIX_EPOCH)
                    .map_or(0, |d| d.as_millis());
                dir.join(format!(
                    "{PREFIX}{millis:013}-{}-{:010}{SUFFIX}",
                    std::process::id(),
                    SEQ.fetch_add(1, Ordering::Relaxed)
                ))
            }
        };
        if let Some(dir) = path.parent() {
            create_private_dirs(dir)?;
        }
        let mut options = OpenOptions::new();
        options.write(true).create_new(true);
        #[cfg(unix)]
        std::os::unix::fs::OpenOptionsExt::mode(&mut options, 0o600);
        let file = options.open(&path)?;
        Ok(RawLog {
            path,
            file: Some(file),
            cut: Default::default(),
        })
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    /// The path of a file that goes with the log at `log`: `<stem>.<what>.log`
    /// beside `<stem>.<ext>`, `console.collapsed-1.log` beside `console.log`.
    pub fn sibling(log: &Path, what: &str) -> PathBuf {
        let stem = log
            .file_stem()
            .and_then(|n| n.to_str())
            .unwrap_or("console");
        log.with_file_name(format!("{stem}.{what}{SUFFIX}"))
    }

    /// Why the log has stopped short of the console, as a [`Cut`] code, for
    /// whoever names it.
    pub fn cut(&self) -> std::sync::Arc<std::sync::atomic::AtomicU8> {
        self.cut.clone()
    }

    /// Append `bytes`. A write that fails closes the log; the console itself
    /// is unaffected.
    pub fn write(&mut self, bytes: &[u8]) {
        let Some(file) = self.file.as_mut() else {
            return;
        };
        if let Err(e) = file.write_all(bytes) {
            tracing::debug!("raw console log {} failed: {e}", self.path.display());
            self.file = None;
            self.cut
                .store(Cut::Failed as u8, std::sync::atomic::Ordering::Relaxed);
        }
    }
}

/// Open `path` to append to, creating it `0600`.
pub(crate) fn append_private(path: &Path) -> io::Result<File> {
    let mut options = OpenOptions::new();
    options.append(true).create(true);
    #[cfg(unix)]
    std::os::unix::fs::OpenOptionsExt::mode(&mut options, 0o600);
    options.open(path)
}

/// Create `dir` and any parents it lacks, each `0700`; one that exists keeps
/// its mode.
fn create_private_dirs(dir: &Path) -> io::Result<()> {
    let missing: Vec<&Path> = dir.ancestors().take_while(|d| !d.is_dir()).collect();
    for d in missing.into_iter().rev() {
        match fs::create_dir(d) {
            Ok(()) => restrict(d, 0o700),
            Err(e) if e.kind() == io::ErrorKind::AlreadyExists => {}
            Err(e) => return Err(e),
        }
    }
    Ok(())
}

fn restrict(_dir: &Path, _mode: u32) {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let _ = fs::set_permissions(_dir, fs::Permissions::from_mode(_mode));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_log_holds_what_is_written_and_is_private() {
        let dir = tempfile::tempdir().unwrap();
        let mut log = RawLog::create(&Place::In(dir.path().to_path_buf())).unwrap();
        log.write(b"one\n");
        log.write(b"two\n");
        let path = log.path().to_path_buf();
        drop(log);
        assert_eq!(fs::read(&path).unwrap(), b"one\ntwo\n");
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = fs::metadata(&path).unwrap().permissions().mode();
            assert_eq!(mode & 0o777, 0o600);
        }
    }

    #[cfg(unix)]
    #[test]
    fn only_a_directory_made_for_logs_is_made_private() {
        use std::os::unix::fs::PermissionsExt;
        let dir = tempfile::tempdir().unwrap();
        fs::set_permissions(dir.path(), fs::Permissions::from_mode(0o1777)).unwrap();
        RawLog::create(&Place::In(dir.path().to_path_buf())).unwrap();
        let mode = |p: &Path| fs::metadata(p).unwrap().permissions().mode() & 0o7777;
        assert_eq!(
            mode(dir.path()),
            0o1777,
            "an existing directory keeps its mode"
        );
        let made = dir.path().join("a/b");
        RawLog::create(&Place::In(made.clone())).unwrap();
        assert_eq!(mode(&made), 0o700);
        assert_eq!(mode(&dir.path().join("a")), 0o700, "its parents too");
    }

    /// A log of its own goes at exactly its path, which must not exist yet,
    /// and its siblings beside it.
    #[test]
    fn a_log_at_a_path_is_made_there_once() {
        let dir = tempfile::tempdir().unwrap();
        let at = dir.path().join("bazel/1/console.log");
        let log = RawLog::create(&Place::At(at.clone())).unwrap();
        assert_eq!(log.path(), at);
        assert!(
            RawLog::create(&Place::At(at.clone())).is_err(),
            "never over another"
        );
        assert_eq!(
            RawLog::sibling(&at, "collapsed-1"),
            dir.path().join("bazel/1/console.collapsed-1.log")
        );
        assert_eq!(
            RawLog::sibling(Path::new("/c/out.txt"), "collapsed-1"),
            Path::new("/c/out.collapsed-1.log"),
            "named after the log whatever its extension"
        );
    }

    #[test]
    fn a_siblings_name_starts_with_its_logs() {
        let log = Path::new("/c/console-0000000000001-7-0000000003.log");
        assert_eq!(
            RawLog::sibling(log, "collapsed-2"),
            Path::new("/c/console-0000000000001-7-0000000003.collapsed-2.log")
        );
    }
}
