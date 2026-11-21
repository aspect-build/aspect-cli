//! `task.json`: a task invocation's own record in its history entry (see
//! [`super::history`]), what a history list, a re-run or a browser reads
//! without opening a build's files.
//!
//! It is written when the task starts (`status: running`, with this
//! process's pid, so a run that crashed is told by a pid no longer alive),
//! again as each Bazel invocation ends, and once more when the task ends,
//! each time whole, through a temp file renamed over it, so a reader never
//! sees half of one. Like everything in the entry it is readable by its user
//! alone (`0600`).
//!
//! The command line is kept as it was typed (`argv`), since running it again
//! needs it whole; whatever shows it (`ctx.std.history`, `aspect history`)
//! redacts it first ([`redact_argv`]): the header and env-passthrough rules
//! the build event stream gets, and each flag the task or a feature declared
//! secret (`args.string(secret = True)`), whose spellings are kept beside it
//! (`secret_flags`). A Bazel invocation's flags are kept redacted. Once a
//! prune removes the entry's other files ([`TaskRecord::evict`]), the
//! command line is redacted in place, so a secret outlives the run's files
//! by no more than they did.
//!
//! Schema version [`SCHEMA`]; a reader skips a record with a newer one.

use std::fs;
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::mpsc::{self, Receiver};
use std::sync::{Arc, Mutex};
use std::thread::JoinHandle;
use std::time::{Duration, SystemTime};

use bazel::bes::sink::summary::Summary;
use serde::{Deserialize, Serialize};

/// The record's file name in an entry.
pub const FILE_NAME: &str = "task.json";

/// The schema this writes.
pub const SCHEMA: u32 = 1;

/// How long the task's end waits for the commit and branch read at its
/// start; whether the tree was dirty is taken only if it has come by then,
/// since `git status` can take seconds in a large repository.
const GIT_WAIT: Duration = Duration::from_millis(100);

/// A task invocation's record.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct TaskRecord {
    pub schema: u32,
    /// `running`, then `ended`.
    pub status: String,
    pub cli_version: String,
    pub task: TaskIdentity,
    /// The process that ran it: one no longer alive with `status: running`
    /// ended without writing its end.
    pub pid: u32,
    /// The command line as typed, the program first; redacted once
    /// `argv_redacted`.
    pub argv: Vec<String>,
    #[serde(default)]
    pub argv_redacted: bool,
    /// The flag spellings whose values are secrets (`--token`, `-t`,
    /// `--feature:token`).
    #[serde(default)]
    pub secret_flags: Vec<String>,
    /// The short flags that take a value but are not secrets (`-m`), to tell
    /// a cluster's value from a secret flag in it.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub value_shorts: Vec<String>,
    pub cwd: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub workspace: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub git: Option<Git>,
    /// RFC 3339, UTC, in milliseconds.
    pub started: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ended: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub duration_ms: Option<u64>,
    /// The task's exit code; absent while it runs, and for a task that
    /// failed with an error (`error`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub exit_code: Option<u8>,
    /// The line the task ended on (`Successful build`).
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub conclusion: String,
    /// The error a task that failed outright ended with.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub error: String,
    #[serde(default)]
    pub cancelled: bool,
    /// Results pages and the like (`ctx.std.history.link`).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub links: Vec<Link>,
    /// Each Bazel invocation, in order.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub bazel: Vec<BazelRecord>,
    /// When a prune removed the entry's other files, RFC 3339.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub evicted: Option<String>,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct TaskIdentity {
    pub id: String,
    pub name: String,
    pub kind: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub group: Vec<String>,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct Git {
    pub commit: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub branch: Option<String>,
    /// Whether tracked files differed from the commit; absent when that had
    /// not been told by the task's end.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub dirty: Option<bool>,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct Link {
    pub label: String,
    pub url: String,
}

/// One Bazel invocation.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct BazelRecord {
    /// The Bazel command (`build`, `test`).
    pub command: String,
    pub patterns: Vec<String>,
    /// Its flags, redacted.
    pub flags: Vec<String>,
    /// Its folder in the entry (`bazel/1`), when it kept files.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub folder: Option<String>,
    pub started: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ended: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub exit_code: Option<i32>,
    /// The id every gRPC BES sink indexed it under.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub sink_invocation_id: String,
    /// What its build event stream said, when it had one.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub summary: Option<Summary>,
}

impl TaskRecord {
    /// Forget the run's files: the command line redacted, the time noted.
    pub fn evict(&mut self, at: SystemTime) {
        if !self.argv_redacted {
            self.argv = redact_argv(&self.argv, &self.secret_flags, &self.value_shorts);
            self.argv_redacted = true;
        }
        for b in &mut self.bazel {
            if let Some(s) = b.summary.as_mut() {
                for f in &mut s.failures {
                    f.logs.clear();
                }
            }
        }
        self.evicted = Some(timestamp(at));
    }

    /// The record as shown: its command line redacted.
    pub fn redacted(&self) -> TaskRecord {
        let mut shown = self.clone();
        if !shown.argv_redacted {
            shown.argv = redact_argv(&shown.argv, &shown.secret_flags, &shown.value_shorts);
            shown.argv_redacted = true;
        }
        shown
    }
}

/// The shortest secret value scrubbed wherever it appears ([`scrub`]):
/// shorter ones would match too much text that is not them.
const MIN_SCRUBBED: usize = 4;

/// `argv` with secrets redacted: the value of each of `secret_flags`, as
/// `--flag=value`, `--flag value`, `-f value`, `-fvalue`, or in a cluster of
/// short flags (`-vf value`, `-vfvalue`); only `--flag=value` and `-f=value`
/// for one spelled with a trailing `=`, a flag that may be given bare; each
/// such value anywhere else in it
/// too ([`scrub`]), as when a task passes it on in another flag; then
/// everything by the build event stream's rules (`bazel::bes::redact`). After
/// a bare `--` only the scrub and those rules apply.
pub fn redact_argv(
    argv: &[String],
    secret_flags: &[String],
    value_shorts: &[String],
) -> Vec<String> {
    let (out, values) = split_secrets(argv, secret_flags, value_shorts);
    let out = scrub(&out, &values);
    bazel::bes::redact::redact_command_args(out.iter().map(String::as_str))
}

/// The values `argv` gives the flags in `secret_flags`, in every spelling
/// [`redact_argv`] takes.
pub fn secret_values(
    argv: &[String],
    secret_flags: &[String],
    value_shorts: &[String],
) -> Vec<String> {
    split_secrets(argv, secret_flags, value_shorts).1
}

/// Each of `args` with every one of `values` (those of at least
/// [`MIN_SCRUBBED`] characters) in it replaced by the redaction marker.
pub fn scrub(args: &[String], values: &[String]) -> Vec<String> {
    let redacted = bazel::bes::redact::REDACTED;
    let mut values: Vec<&String> = values
        .iter()
        .filter(|v| v.chars().count() >= MIN_SCRUBBED)
        .collect();
    // Longest first, so a value inside another is not replaced first.
    values.sort_by_key(|v| std::cmp::Reverse(v.len()));
    args.iter()
        .map(|arg| {
            let mut arg = arg.clone();
            for v in &values {
                if arg.contains(v.as_str()) {
                    arg = arg.replace(v.as_str(), redacted);
                }
            }
            arg
        })
        .collect()
}

/// `argv` with the values of `secret_flags` replaced, and those values. A
/// spelling ending in `=` (a flag that may be given bare) takes its value
/// only after `=`, never from the next argument. `value_shorts` are the
/// other short flags that take a value (`-m`): in a cluster, one takes the
/// rest of it, as the parser reads it, so no secret flag follows it there.
fn split_secrets(
    argv: &[String],
    secret_flags: &[String],
    value_shorts: &[String],
) -> (Vec<String>, Vec<String>) {
    let redacted = bazel::bes::redact::REDACTED;
    let spelled = |f: &String| (f.trim_end_matches('=').to_owned(), !f.ends_with('='));
    let long: Vec<(String, bool)> = secret_flags
        .iter()
        .filter(|f| f.starts_with("--"))
        .map(spelled)
        .collect();
    let short: Vec<(char, bool)> = secret_flags
        .iter()
        .filter(|f| !f.starts_with("--"))
        .filter_map(|f| {
            let (flag, next) = spelled(f);
            let mut c = flag.strip_prefix('-')?.chars();
            let only = c.next()?;
            c.next().is_none().then_some((only, next))
        })
        .collect();
    let mut out = Vec::with_capacity(argv.len());
    let mut values = vec![];
    let mut takes_value = false;
    let mut ended = false;
    for arg in argv {
        if takes_value {
            takes_value = false;
            values.push(arg.clone());
            out.push(redacted.to_string());
            continue;
        }
        if ended || arg == "--" {
            ended = true;
            out.push(arg.clone());
            continue;
        }
        if let Some((_, next)) = long.iter().find(|(f, _)| arg == f) {
            takes_value = *next;
            out.push(arg.clone());
            continue;
        }
        let with_eq = long.iter().find_map(|(f, _)| {
            arg.strip_prefix(f.as_str())
                .and_then(|r| r.strip_prefix('='))
                .map(|v| (f, v))
        });
        if let Some((flag, value)) = with_eq {
            values.push(value.to_owned());
            out.push(format!("{flag}={redacted}"));
            continue;
        }
        // A cluster of short flags (letters and digits up to the secret one,
        // so a negative pattern like `-//pkg/test:all` is none): the first
        // secret one takes the rest of the token as its value (an `=` before
        // it dropped), or, at the end, the next token.
        let cluster = arg
            .strip_prefix('-')
            .filter(|c| !c.is_empty() && !c.starts_with('-'));
        let found = cluster.and_then(|cluster| {
            cluster
                .char_indices()
                .take_while(|(_, c)| c.is_ascii_alphanumeric())
                // Up to and including the first that takes a value: the rest
                // is its value.
                .scan(false, |past_value, (at, c)| {
                    if *past_value {
                        return None;
                    }
                    *past_value = value_shorts
                        .iter()
                        .any(|v| v.strip_prefix('-') == Some(c.encode_utf8(&mut [0; 4])));
                    Some((at, c))
                })
                .find_map(|(at, c)| {
                    short
                        .iter()
                        .find(|(s, _)| *s == c)
                        .map(|(_, next)| (cluster, at, c, *next))
                })
        });
        if let Some((cluster, at, c, next)) = found {
            let head = &arg[..1 + at + c.len_utf8()];
            let rest = &cluster[at + c.len_utf8()..];
            if rest.is_empty() {
                takes_value = next;
                out.push(arg.clone());
            } else if !next && !rest.starts_with('=') {
                // One that may be bare: the letters after it are other flags.
                out.push(arg.clone());
            } else {
                let (eq, value) = match rest.strip_prefix('=') {
                    Some(v) => ("=", v),
                    None => ("", rest),
                };
                values.push(value.to_owned());
                out.push(format!("{head}{eq}{redacted}"));
            }
            continue;
        }
        out.push(arg.clone());
    }
    (out, values)
}

/// Whether a process `pid` is alive on this machine (one this user may not
/// signal counts).
pub fn alive(pid: u32) -> bool {
    #[cfg(unix)]
    {
        let Ok(pid) = i32::try_from(pid) else {
            return false;
        };
        // Signal 0 checks for the process without sending anything.
        match nix::sys::signal::kill(nix::unistd::Pid::from_raw(pid), None) {
            Ok(()) | Err(nix::errno::Errno::EPERM) => true,
            Err(_) => false,
        }
    }
    #[cfg(not(unix))]
    {
        let _ = pid;
        true
    }
}

/// How long a record may say `running` without being written before its
/// run is taken to have ended without saying so, though a process has its
/// pid: pids are reused, and a crash's would keep its files forever.
const RUNNING_IDLE: Duration = Duration::from_secs(7 * 24 * 60 * 60);

/// Whether the run of `record`, last written at `written`, is still going:
/// it says `running`, from a process other than this one (whose own run is
/// in no other entry), alive, and it was written within [`RUNNING_IDLE`].
pub fn still_running(record: &TaskRecord, written: Option<SystemTime>) -> bool {
    let recent = written.is_some_and(|t| {
        SystemTime::now()
            .duration_since(t)
            .map_or(true, |ago| ago < RUNNING_IDLE)
    });
    record.status == "running" && record.pid != std::process::id() && alive(record.pid) && recent
}

/// The message of an error a task ended with, without the traceback its
/// display leads with: its `error: ` line, else its first.
pub fn error_line(text: &str) -> String {
    text.lines()
        .find_map(|l| l.strip_prefix("error: "))
        .or_else(|| text.lines().find(|l| !l.trim().is_empty()))
        .unwrap_or("")
        .trim()
        .to_owned()
}

/// `time` as RFC 3339 in UTC, to the millisecond.
pub fn timestamp(time: SystemTime) -> String {
    chrono::DateTime::<chrono::Utc>::from(time).to_rfc3339_opts(chrono::SecondsFormat::Millis, true)
}

/// A record on disk, kept up to date as the task goes.
#[derive(Debug)]
pub struct RecordFile {
    path: PathBuf,
    record: Mutex<TaskRecord>,
    started: SystemTime,
    /// The git details, read beside the task (see [`read_git`]): the commit
    /// and branch, then whether the tree was dirty.
    git: Mutex<Option<Receiver<GitPart>>>,
}

impl RecordFile {
    /// Start the record at `dir/task.json`, written now, the git details of
    /// `git_root` read in the background.
    pub fn begin(
        dir: &Path,
        record: TaskRecord,
        git_root: Option<PathBuf>,
    ) -> io::Result<Arc<Self>> {
        let git = git_root.map(|root| {
            let (tx, rx) = mpsc::channel();
            let _ = std::thread::Builder::new()
                .name("history-git".into())
                .spawn(move || read_git(&root, &tx));
            rx
        });
        let file = Arc::new(Self {
            path: dir.join(FILE_NAME),
            record: Mutex::new(record),
            started: SystemTime::now(),
            git: Mutex::new(git),
        });
        file.save()?;
        Ok(file)
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    /// Change the record and write it.
    pub fn update(&self, f: impl FnOnce(&mut TaskRecord)) {
        f(&mut self.record.lock().unwrap());
        if let Err(e) = self.save() {
            tracing::debug!("could not write {}: {e}", self.path.display());
        }
    }

    /// A copy of the record as it stands.
    pub fn snapshot(&self) -> TaskRecord {
        self.record.lock().unwrap().clone()
    }

    /// The task's end: how it ended, the git details if they have come.
    pub fn end(&self, exit_code: Option<u8>, conclusion: String, error: String, cancelled: bool) {
        let git = self.git.lock().unwrap().take().and_then(|rx| {
            let GitPart::Head(mut git) = rx.recv_timeout(GIT_WAIT).ok()? else {
                return None;
            };
            if let Ok(GitPart::Dirty(dirty)) = rx.try_recv() {
                git.dirty = Some(dirty);
            }
            Some(git)
        });
        let now = SystemTime::now();
        let duration = now.duration_since(self.started).unwrap_or_default();
        self.update(|r| {
            r.status = "ended".into();
            r.ended = Some(timestamp(now));
            r.duration_ms = Some(duration.as_millis() as u64);
            r.exit_code = exit_code;
            r.conclusion = conclusion;
            r.error = error;
            r.cancelled = cancelled;
            if git.is_some() {
                r.git = git;
            }
        });
    }

    fn save(&self) -> io::Result<()> {
        let text = serde_json::to_vec_pretty(&*self.record.lock().unwrap())?;
        write_private(&self.path, &text)
    }
}

/// Replace `path` whole with `bytes`, readable by this user alone, through a
/// temp file beside it renamed over it.
pub fn write_private(path: &Path, bytes: &[u8]) -> io::Result<()> {
    let temp = path.with_extension(format!("{}.tmp", uuid::Uuid::new_v4()));
    let result = (|| {
        let mut options = fs::OpenOptions::new();
        options.write(true).create_new(true);
        #[cfg(unix)]
        std::os::unix::fs::OpenOptionsExt::mode(&mut options, 0o600);
        let mut file = options.open(&temp)?;
        file.write_all(bytes)?;
        fs::rename(&temp, path)
    })();
    if result.is_err() {
        let _ = fs::remove_file(&temp);
    }
    result
}

/// Read the record at `path`; None when it is missing, unreadable, or of a
/// newer schema.
pub fn read(path: &Path) -> Option<TaskRecord> {
    let record: TaskRecord = serde_json::from_slice(&fs::read(path).ok()?).ok()?;
    (record.schema <= SCHEMA).then_some(record)
}

/// What [`read_git`] sends, in this order.
#[derive(Debug)]
enum GitPart {
    Head(Git),
    Dirty(bool),
}

/// Send the repository's commit and branch, then whether tracked files
/// differ, from `git`; nothing outside a repository or without git. Run
/// beside the task, so it never delays its start.
fn read_git(root: &Path, tx: &mpsc::Sender<GitPart>) {
    let git = |args: &[&str]| -> Option<String> {
        // No optional locks: `git status` would otherwise take the index's
        // lock to refresh it, and a `git commit` of the user's at once fail.
        let out = Command::new("git")
            .env("GIT_OPTIONAL_LOCKS", "0")
            .arg("-C")
            .arg(root)
            .args(args)
            .stdin(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .output()
            .ok()?;
        out.status
            .success()
            .then(|| String::from_utf8_lossy(&out.stdout).into_owned())
    };
    // One call for both: the commit, then `HEAD`'s ref (`HEAD` itself when
    // detached).
    let Some(head) = git(&[
        "rev-parse",
        "--verify",
        "HEAD",
        "--symbolic-full-name",
        "HEAD",
    ]) else {
        return;
    };
    let mut lines = head.lines();
    let Some(commit) = lines.next().map(str::trim).filter(|c| !c.is_empty()) else {
        return;
    };
    let branch = lines
        .next()
        .and_then(|r| r.trim().strip_prefix("refs/heads/"))
        .map(str::to_owned);
    let _ = tx.send(GitPart::Head(Git {
        commit: commit.to_owned(),
        branch,
        dirty: None,
    }));
    if let Some(status) = git(&["status", "--porcelain", "--untracked-files=no"]) {
        let _ = tx.send(GitPart::Dirty(!status.trim().is_empty()));
    }
}

/// A Bazel invocation's place in the record, kept up to date as it ends.
#[derive(Debug)]
pub struct BazelSlot {
    file: Arc<RecordFile>,
    index: usize,
    /// Its history folder, where the kept logs' indexes are.
    folder: Option<PathBuf>,
    /// The reader of its summary, joined once its stream is over.
    tally: Mutex<Option<JoinHandle<Summary>>>,
    /// The sink keeping its logs, whose indexes the summary's failures are
    /// matched with: joined first.
    keep: Mutex<Option<JoinHandle<bazel::fanout::SinkOutcome>>>,
}

impl BazelSlot {
    pub fn new(file: Arc<RecordFile>, record: BazelRecord, folder: Option<PathBuf>) -> Arc<Self> {
        let mut index = 0;
        file.update(|r| {
            index = r.bazel.len();
            r.bazel.push(record);
        });
        Arc::new(Self {
            file,
            index,
            folder,
            tally: Mutex::new(None),
            keep: Mutex::new(None),
        })
    }

    /// The sink keeping its logs, to join before matching failures with them.
    pub fn keep_with(&self, keep: JoinHandle<bazel::fanout::SinkOutcome>) {
        *self.keep.lock().unwrap() = Some(keep);
    }

    /// Read its summary from `tally`.
    pub fn summarize_from(&self, tally: JoinHandle<Summary>) {
        *self.tally.lock().unwrap() = Some(tally);
    }

    /// Its client exited with `exit_code`.
    pub fn exited(&self, exit_code: Option<i32>, sink_invocation_id: String) {
        let now = timestamp(SystemTime::now());
        self.file.update(|r| {
            if let Some(b) = r.bazel.get_mut(self.index)
                && b.ended.is_none()
            {
                b.ended = Some(now);
                b.exit_code = exit_code;
                b.sink_invocation_id = sink_invocation_id;
            }
        });
    }

    /// Its stream is over: the keep sink joined, then the summary, its
    /// failures given the logs the history kept. With `only_if_done`, a
    /// summary or a keep sink still at work is left (the task is ending
    /// without waiting for the build).
    pub fn summarize(&self, only_if_done: bool) {
        let mut tally = self.tally.lock().unwrap();
        let mut keep = self.keep.lock().unwrap();
        if only_if_done
            && (!tally.as_ref().is_some_and(|t| t.is_finished())
                || keep.as_ref().is_some_and(|k| !k.is_finished()))
        {
            return;
        }
        if let Some(keep) = keep.take() {
            let _ = keep.join();
        }
        drop(keep);
        let Some(Ok(mut summary)) = tally.take().map(JoinHandle::join) else {
            return;
        };
        drop(tally);
        if let Some(folder) = &self.folder {
            attach_logs(&mut summary, folder);
        }
        self.file.update(|r| {
            if let Some(b) = r.bazel.get_mut(self.index) {
                b.summary = Some(summary);
            }
        });
    }
}

/// Give each failure the logs its build's history folder kept for it, from
/// the indexes there (`actions.json`, `testlogs.json`), as paths relative to
/// the task's entry. An action's are matched by label and mnemonic in order,
/// a test attempt's by label, run, shard and attempt.
fn attach_logs(summary: &mut Summary, folder: &Path) {
    let Some(entry) = folder.parent().and_then(Path::parent) else {
        return;
    };
    let rel = |p: &str| {
        folder
            .join(p)
            .strip_prefix(entry)
            .map(|p| p.to_string_lossy().into_owned())
            .ok()
    };
    let index = |name: &str, key: &str| -> Vec<serde_json::Value> {
        fs::read(folder.join(name))
            .ok()
            .and_then(|b| serde_json::from_slice::<serde_json::Value>(&b).ok())
            .and_then(|v| v.get(key).and_then(|a| a.as_array()).cloned())
            .unwrap_or_default()
    };
    let mut actions: Vec<serde_json::Value> = index("actions.json", "actions")
        .into_iter()
        .filter(|a| a.get("success").and_then(|s| s.as_bool()) == Some(false))
        .collect();
    let tests = index("testlogs.json", "tests");
    let str_of =
        |v: &serde_json::Value, k: &str| v.get(k).and_then(|s| s.as_str()).unwrap_or("").to_owned();
    let int_of = |v: &serde_json::Value, k: &str| v.get(k).and_then(|s| s.as_i64()).unwrap_or(0);
    for f in &mut summary.failures {
        match f.kind.as_str() {
            "action" => {
                let at = actions.iter().position(|a| {
                    str_of(a, "label") == f.label
                        && (str_of(a, "mnemonic") == f.detail
                            || (str_of(a, "mnemonic").is_empty() && f.detail == "Action"))
                });
                if let Some(at) = at {
                    let a = actions.remove(at);
                    for stream in ["stderr", "stdout"] {
                        if let Some(p) = a.get(stream).and_then(|s| s.as_str()).and_then(rel) {
                            f.logs.push(p);
                        }
                    }
                }
            }
            "test" => {
                let Some((run, shard, attempt)) = f.attempt else {
                    continue;
                };
                let found = tests.iter().find(|t| {
                    str_of(t, "label") == f.label
                        && int_of(t, "run") == run as i64
                        && int_of(t, "shard") == shard as i64
                        && int_of(t, "attempt") == attempt as i64
                });
                if let Some(files) = found
                    .and_then(|t| t.get("files"))
                    .and_then(|m| m.as_object())
                {
                    let mut names: Vec<&String> = files.keys().collect();
                    names.sort_by_key(|n| (n.as_str() != "test.log", n.as_str().to_owned()));
                    for name in names {
                        if let Some(p) = files[name].as_str().and_then(rel) {
                            f.logs.push(p);
                        }
                    }
                }
            }
            _ => {}
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn s(v: &[&str]) -> Vec<String> {
        v.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn secret_flags_are_redacted_in_every_spelling() {
        let secrets = s(&["--token", "-t", "--deploy:key"]);
        let argv = s(&[
            "aspect",
            "deploy",
            "--token=abc",
            "--token",
            "def",
            "-t",
            "ghi",
            "-tjkl",
            "--tokens=keep",
            "--deploy:key=mno",
            "--remote_header",
            "Authorization: Bearer x",
            "--",
            "--token=passed-on",
        ]);
        let r = bazel::bes::redact::REDACTED;
        assert_eq!(
            redact_argv(&argv, &secrets, &[]),
            vec![
                "aspect".to_string(),
                "deploy".into(),
                format!("--token={r}"),
                "--token".into(),
                r.into(),
                "-t".into(),
                r.into(),
                format!("-t{r}"),
                "--tokens=keep".into(),
                format!("--deploy:key={r}"),
                "--remote_header".into(),
                format!("Authorization:{r}"),
                "--".into(),
                "--token=passed-on".into(),
            ]
        );
    }

    /// A secret short flag in a cluster, and the secret's value passed on
    /// elsewhere on the command line, as another flag's.
    #[test]
    fn a_secret_is_redacted_in_a_cluster_and_wherever_it_is_passed_on() {
        let secrets = s(&["-t", "--token"]);
        let r = bazel::bes::redact::REDACTED;
        assert_eq!(
            redact_argv(
                &s(&[
                    "x",
                    "-vtS3CRET",
                    "-vt",
                    "OTHER1",
                    "-t=EQUALS",
                    "--bazel-flag=--define=T=S3CRET"
                ]),
                &secrets,
                &[]
            ),
            vec![
                "x".to_string(),
                format!("-vt{r}"),
                "-vt".into(),
                r.into(),
                format!("-t={r}"),
                format!("--bazel-flag=--define=T={r}"),
            ]
        );
        assert_eq!(
            secret_values(&s(&["--token", "abcd", "-tefgh"]), &secrets, &[]),
            ["abcd", "efgh"]
        );
        assert_eq!(
            secret_values(&s(&["-//pkg/test:all", "--", "-tx"]), &secrets, &[]),
            Vec::<String>::new(),
            "a negative pattern is no cluster, and nothing after `--` is a flag"
        );
        let bare = s(&["--token=", "-t="]);
        assert_eq!(
            redact_argv(
                &s(&[
                    "x",
                    "--token",
                    "//app:all",
                    "--token=abcd",
                    "-tv",
                    "-t=efgh"
                ]),
                &bare,
                &[]
            ),
            vec![
                "x".to_string(),
                "--token".into(),
                "//app:all".into(),
                format!("--token={r}"),
                "-tv".into(),
                format!("-t={r}"),
            ],
            "a flag that may be bare takes a value only after `=`"
        );
        assert_eq!(
            redact_argv(
                &s(&["deploy", "-mtest-release", "-vmt"]),
                &secrets,
                &s(&["-m"])
            ),
            vec!["deploy".to_string(), "-mtest-release".into(), "-vmt".into()],
            "a value-taking short flag takes the rest of its cluster"
        );
        assert_eq!(
            scrub(&s(&["--define=K=abcd", "a=abc"]), &s(&["abcd", "abc"])),
            vec![format!("--define=K={r}"), "a=abc".into()],
            "too short a value is left alone"
        );
    }

    #[test]
    fn a_record_is_written_whole_and_private() {
        let dir = tempfile::tempdir().unwrap();
        let record = TaskRecord {
            schema: SCHEMA,
            status: "running".into(),
            argv: s(&["aspect", "build", "--token=x"]),
            secret_flags: s(&["--token"]),
            ..Default::default()
        };
        let file = RecordFile::begin(dir.path(), record, None).unwrap();
        file.end(Some(3), "Build failed".into(), String::new(), false);
        let read = read(file.path()).unwrap();
        assert_eq!(
            (
                read.status.as_str(),
                read.exit_code,
                read.conclusion.as_str()
            ),
            ("ended", Some(3), "Build failed")
        );
        assert_eq!(read.argv[2], "--token=x", "kept as typed");
        assert!(read.redacted().argv[2].contains("REDACTED"));
        let leftovers: Vec<_> = fs::read_dir(dir.path())
            .unwrap()
            .map(|e| e.unwrap().file_name())
            .collect();
        assert_eq!(leftovers, [std::ffi::OsString::from(FILE_NAME)]);
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = fs::metadata(file.path()).unwrap().permissions().mode();
            assert_eq!(mode & 0o777, 0o600);
        }
    }

    #[cfg(unix)]
    #[test]
    fn a_run_is_running_only_from_another_live_process_written_lately() {
        let running = |pid| TaskRecord {
            status: "running".into(),
            pid,
            ..Default::default()
        };
        let now = Some(SystemTime::now());
        // The parent stands in for another live process.
        let parent = std::os::unix::process::parent_id();
        assert!(still_running(&running(parent), now));
        assert!(
            !still_running(&running(std::process::id()), now),
            "this process's own pid, reused"
        );
        let idle = Some(SystemTime::now() - RUNNING_IDLE);
        assert!(!still_running(&running(parent), idle), "silent too long");
        assert!(!still_running(&running(parent), None));
        let ended = TaskRecord {
            pid: parent,
            status: "ended".into(),
            ..Default::default()
        };
        assert!(!still_running(&ended, now));
    }

    #[test]
    fn an_error_is_its_message_not_its_traceback() {
        let text = "Traceback (most recent call last):\n  * a.axl:3, in _impl\nerror: fail: boom\n --> a.axl:3:5\n";
        assert_eq!(error_line(text), "fail: boom");
        assert_eq!(error_line("plain\nmore"), "plain");
    }

    #[test]
    fn a_newer_schema_is_skipped() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join(FILE_NAME);
        fs::write(&path, br#"{"schema": 99}"#).unwrap();
        assert_eq!(read(&path), None);
        fs::write(&path, b"not json").unwrap();
        assert_eq!(read(&path), None);
    }

    #[test]
    fn eviction_redacts_the_command_line_and_drops_log_paths() {
        let mut record = TaskRecord {
            argv: s(&["aspect", "--token=x"]),
            secret_flags: s(&["--token"]),
            bazel: vec![BazelRecord {
                summary: Some(Summary {
                    failures: vec![bazel::bes::sink::summary::Failure {
                        kind: "action".into(),
                        label: "//a".into(),
                        detail: "Genrule".into(),
                        attempt: None,
                        logs: s(&["bazel/1/actions/a.stderr"]),
                    }],
                    ..Default::default()
                }),
                ..Default::default()
            }],
            ..Default::default()
        };
        record.evict(SystemTime::UNIX_EPOCH);
        assert!(record.argv_redacted && record.argv[1].contains("REDACTED"));
        assert!(
            record.bazel[0].summary.as_ref().unwrap().failures[0]
                .logs
                .is_empty()
        );
        assert_eq!(record.evicted.as_deref(), Some("1970-01-01T00:00:00.000Z"));
    }

    #[test]
    fn failures_get_the_logs_the_history_kept() {
        let entry = tempfile::tempdir().unwrap();
        let folder = entry.path().join("bazel/1");
        fs::create_dir_all(&folder).unwrap();
        fs::write(
            folder.join("actions.json"),
            serde_json::to_vec(&serde_json::json!({"actions": [
                {"label": "//a", "mnemonic": "Genrule", "success": true, "stderr": "actions/a/Genrule-1.stderr"},
                {"label": "//a", "mnemonic": "Genrule", "success": false, "stderr": "actions/a/Genrule-2.stderr", "stdout": "actions/a/Genrule-2.stdout"},
            ]}))
            .unwrap(),
        )
        .unwrap();
        fs::write(
            folder.join("testlogs.json"),
            serde_json::to_vec(&serde_json::json!({"tests": [
                {"label": "//t", "run": 1, "shard": 2, "attempt": 1, "files": {"test.xml": "testlogs/t/x", "test.log": "testlogs/t/l"}},
            ]}))
            .unwrap(),
        )
        .unwrap();
        let failure =
            |kind: &str, label: &str, detail: &str, attempt| bazel::bes::sink::summary::Failure {
                kind: kind.into(),
                label: label.into(),
                detail: detail.into(),
                attempt,
                logs: vec![],
            };
        let mut summary = Summary {
            failures: vec![
                failure("action", "//a", "Genrule", None),
                failure("test", "//t", "shard 2 · attempt 1", Some((1, 2, 1))),
                failure("test", "//t", "attempt 2", Some((1, 2, 2))),
            ],
            ..Default::default()
        };
        attach_logs(&mut summary, &folder);
        let logs: Vec<&Vec<String>> = summary.failures.iter().map(|f| &f.logs).collect();
        assert_eq!(
            logs,
            [
                &s(&[
                    "bazel/1/actions/a/Genrule-2.stderr",
                    "bazel/1/actions/a/Genrule-2.stdout"
                ]),
                &s(&["bazel/1/testlogs/t/l", "bazel/1/testlogs/t/x"]),
                &vec![],
            ]
        );
    }
}
