//! Keeps a build's action and test logs in its history folder, from the BES
//! stream as it arrives.
//!
//! An action's stdout and stderr are files Bazel writes under
//! `bazel-out/_tmp/actions/` and names in the `ActionExecuted` event, and a
//! test's logs are files in the output tree (`testlogs/`, an earlier
//! attempt's under `test_attempts/`); the next build replaces each (a new
//! file at the same name, on Bazel 7, 8 and 9, sandboxed, `local` or
//! `standalone`), never rewriting one in place. Both are hard-linked here,
//! which costs nothing until Bazel replaces its own and leaves Bazel's path
//! alone for anything that still reads it (a BES upload finishing after the
//! client exits), and copied when a link cannot be made (another
//! filesystem). A `file://` log is kept from its path and inline contents
//! are written out. A remote cache names logs by `bytestream://` instead: a test's
//! `test.log` and `test.xml` are then found where Bazel's output tree has
//! them, once the stream ends or the build is retried in it; any other such
//! log (an action's, with a remote cache) is counted as `not_local`.
//!
//! An action's kept logs are also published as they are placed
//! ([`KeptActions`]), for a screen that shows a running build's failures
//! after Bazel has replaced its own files.
//!
//! What is kept is listed in an index per kind beside its folder, out of
//! reach of any label, written when the stream ends; a build the CLI was
//! interrupted out of may have none:
//!
//! ```text
//! actions/[external/<repo>/]<package>/<name>/<mnemonic>-<k>.{stdout,stderr}
//! actions.json
//! testlogs/[external/<repo>/]<package>/<name>/run_<r>/shard_<s>/attempt_<a>/<file>
//! testlogs.json
//! ```
//!
//! A failure to keep one file is counted in the index and never fails the
//! build.

use std::collections::HashMap;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::sync::mpsc::RecvError;
use std::sync::{Arc, Mutex};
use std::thread::{self, JoinHandle};

use axl_proto::build_event_stream::build_event::Payload;
use axl_proto::build_event_stream::build_event_id::{
    ActionCompletedId, ConfigurationId, Id, TestResultId,
};
use axl_proto::build_event_stream::file;
use axl_proto::build_event_stream::{ActionExecuted, File, TestResult, TestStatus};
use serde_json::{Value, json};

use super::events::attempt_cached;
use crate::bes::BuildEventEnvelope;
use crate::fanout::{SinkOutcome, Subscriber, SubscriberFilter};

/// Which actions' logs to keep.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum ActionLogs {
    #[default]
    None,
    /// Those of actions that failed: Bazel reports these by default.
    Failed,
    /// Those of every action that wrote any: Bazel reports them only with
    /// [`PUBLISH_ALL_ACTIONS`](super::downstream::PUBLISH_ALL_ACTIONS), which a
    /// build keeping them is run with unless its flags already ask for it.
    All,
}

/// Which tests' logs to keep.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum TestLogs {
    #[default]
    None,
    /// Those of attempts that did not pass; a flaky one did not.
    Failed,
    /// Those of attempts that ran in this build, not served from a cache.
    Executed,
    /// Every attempt's.
    All,
}

impl ActionLogs {
    pub const NAMES: &str = "none, failed, all";

    pub fn parse(name: &str) -> Option<ActionLogs> {
        Some(match name {
            "none" => ActionLogs::None,
            "failed" => ActionLogs::Failed,
            "all" => ActionLogs::All,
            _ => return None,
        })
    }
}

impl TestLogs {
    pub const NAMES: &str = "none, failed, executed, all";

    pub fn parse(name: &str) -> Option<TestLogs> {
        Some(match name {
            "none" => TestLogs::None,
            "failed" => TestLogs::Failed,
            "executed" => TestLogs::Executed,
            "all" => TestLogs::All,
            _ => return None,
        })
    }
}

/// What a build keeps of its action and test logs.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Keep {
    pub actions: ActionLogs,
    pub tests: TestLogs,
}

/// Each action's kept logs so far, by its primary output: the paths, in the
/// history folder, of its `stdout` and `stderr` that were kept. A retry's
/// action replaces the earlier attempt's.
pub type KeptActions = Arc<Mutex<HashMap<String, KeptAction>>>;

/// An action's kept `stdout` and `stderr`, where each was kept.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct KeptAction {
    pub stdout: Option<PathBuf>,
    pub stderr: Option<PathBuf>,
}

impl Keep {
    /// Whether anything is kept.
    pub fn any(&self) -> bool {
        self.actions != ActionLogs::None || self.tests != TestLogs::None
    }

    /// The events this keeps logs from, for a filtered subscription, so a
    /// copy in progress never holds the rest of the stream in its buffer.
    pub fn filter(self) -> SubscriberFilter<Arc<BuildEventEnvelope>> {
        Arc::new(
            move |envelope: &Arc<BuildEventEnvelope>| match envelope.event.payload.as_ref() {
                Some(Payload::Action(_)) => self.actions != ActionLogs::None,
                // Test logs a remote cache names by `bytestream://` are
                // found where Bazel left them, from these; a retry of the
                // build in one stream starts again with `Started`.
                Some(
                    Payload::TestResult(_)
                    | Payload::TestSummary(_)
                    | Payload::WorkspaceInfo(_)
                    | Payload::Configuration(_)
                    | Payload::Started(_),
                ) => self.tests != TestLogs::None,
                _ => false,
            },
        )
    }

    /// Start keeping logs from `recv` in `dir`, the build's history folder,
    /// each action's as it is kept published to `published`, and for a
    /// failed one an `INFO:` line naming where to `notes` (the console's).
    pub fn spawn(
        self,
        recv: Subscriber<Arc<BuildEventEnvelope>>,
        dir: PathBuf,
        published: KeptActions,
        notes: Option<crate::console::forward::Notes>,
    ) -> JoinHandle<SinkOutcome> {
        thread::spawn(move || {
            let mut kept = Kept::new(self, dir);
            kept.published = Some(published);
            kept.notes = notes;
            loop {
                match recv.recv() {
                    Ok(envelope) => kept.event(&envelope.event),
                    Err(RecvError) => break,
                }
            }
            kept.finish();
            Ok(())
        })
    }
}

/// What a [`Keep`] has kept so far.
struct Kept {
    keep: Keep,
    dir: PathBuf,
    /// Where each action's kept logs are published as they are kept.
    published: Option<KeptActions>,
    /// Where a failed action's kept logs are told, as `INFO:` lines.
    notes: Option<crate::console::forward::Notes>,
    actions: Vec<Value>,
    tests: Vec<Value>,
    /// Per `<label>/<mnemonic>`, how many actions have been kept.
    action_counts: HashMap<String, usize>,
    /// The action logs and the test logs not kept.
    actions_not_kept: NotKept,
    tests_not_kept: NotKept,
    /// Test logs named by a remote URI, found on disk once the stream ends
    /// or the build is retried in it.
    remote_test_logs: Vec<RemoteTestLog>,
    /// For finding them: the exec root, each configuration's output folder
    /// (`darwin_arm64-fastbuild`), each test's `(runs, shards)`, and each
    /// run and shard's last attempt.
    exec_root: Option<PathBuf>,
    output_dirs: HashMap<String, String>,
    test_counts: HashMap<(String, String), (i32, i32)>,
    last_attempts: HashMap<(String, String, i32, i32), i32>,
}

/// A test log named by a remote URI (a remote cache's `bytestream://`),
/// though Bazel keeps it locally too.
struct RemoteTestLog {
    /// Which of `Kept::tests`, and its name there.
    entry: usize,
    name: String,
    /// Where it goes, as for a local one.
    rel: PathBuf,
    label: String,
    config: String,
    run: i32,
    shard: i32,
    attempt: i32,
}

/// Logs not kept, for an index.
#[derive(Default)]
struct NotKept {
    /// Remote (`bytestream://`), and not found on local disk either.
    not_local: usize,
    /// Could not be linked or copied.
    failed: usize,
}

impl NotKept {
    fn json(&self) -> Value {
        json!({ "not_local": self.not_local, "failed": self.failed })
    }
}

impl Kept {
    fn new(keep: Keep, dir: PathBuf) -> Self {
        Self {
            keep,
            dir,
            published: None,
            notes: None,
            actions: vec![],
            tests: vec![],
            action_counts: HashMap::new(),
            actions_not_kept: NotKept::default(),
            tests_not_kept: NotKept::default(),
            remote_test_logs: vec![],
            exec_root: None,
            output_dirs: HashMap::new(),
            test_counts: HashMap::new(),
            last_attempts: HashMap::new(),
        }
    }

    fn event(&mut self, event: &axl_proto::build_event_stream::BuildEvent) {
        match event.payload.as_ref() {
            Some(Payload::WorkspaceInfo(info)) => {
                if !info.local_exec_root.is_empty() {
                    self.exec_root = Some(PathBuf::from(&info.local_exec_root));
                }
                return;
            }
            // A retry of the build (Bazel's after a remote cache eviction)
            // writes the same local test logs again: find the abandoned
            // attempt's while they are still its, and count afresh.
            Some(Payload::Started(_)) => {
                self.find_remote_test_logs();
                self.test_counts.clear();
                self.last_attempts.clear();
                return;
            }
            _ => {}
        }
        let Some(id) = event.id.as_ref().and_then(|w| w.id.as_ref()) else {
            return;
        };
        match (event.payload.as_ref(), id) {
            (Some(Payload::Action(action)), Id::ActionCompleted(id)) => self.action(action, id),
            (Some(Payload::TestResult(result)), Id::TestResult(id)) => self.test(result, id),
            (Some(Payload::TestSummary(summary)), Id::TestSummary(id)) => {
                self.test_counts.insert(
                    (id.label.clone(), config_of(&id.configuration)),
                    (summary.run_count, summary.shard_count),
                );
            }
            (Some(Payload::Configuration(config)), Id::Configuration(id)) => {
                self.output_dirs
                    .insert(id.id.clone(), config.mnemonic.clone());
            }
            _ => {}
        }
    }

    fn action(&mut self, action: &ActionExecuted, id: &ActionCompletedId) {
        let wanted = match self.keep.actions {
            ActionLogs::None => false,
            ActionLogs::Failed => !action.success,
            ActionLogs::All => true,
        };
        if !wanted || (action.stdout.is_none() && action.stderr.is_none()) {
            return;
        }
        // The id's label: the payload's own is deprecated.
        let label = &id.label;
        let mnemonic = if action.r#type.is_empty() {
            "Action"
        } else {
            action.r#type.as_str()
        };
        let base = label_dir(label);
        let key = format!("{}/{mnemonic}", base.display());
        let k = self.action_counts.entry(key).or_default();
        *k += 1;
        let stem = format!("{}-{}", sanitize(mnemonic), k);
        let mut entry = json!({
            "label": label,
            "mnemonic": action.r#type,
            "primary_output": id.primary_output,
            "success": action.success,
            "exit_code": action.exit_code,
        });
        if let Some(start) = action.start_time.as_ref() {
            entry["start"] = json!(timestamp(start));
        }
        if let Some(end) = action.end_time.as_ref() {
            entry["end"] = json!(timestamp(end));
        }
        for (stream, file) in [("stdout", &action.stdout), ("stderr", &action.stderr)] {
            let Some(file) = file else { continue };
            let rel = Path::new("actions")
                .join(&base)
                .join(format!("{stem}.{stream}"));
            if let Some(kept) = keep_file(&self.dir, file, &rel, &mut self.actions_not_kept) {
                entry[stream] = json!(kept);
            }
        }
        if let Some(published) = &self.published {
            let at = |stream: &str| entry[stream].as_str().map(|rel| self.dir.join(rel));
            let kept = KeptAction {
                stdout: at("stdout"),
                stderr: at("stderr"),
            };
            if kept != KeptAction::default() {
                published
                    .lock()
                    .unwrap()
                    .insert(id.primary_output.clone(), kept);
            }
        }
        if !action.success {
            let streams: Vec<(String, String)> = ["stderr", "stdout"]
                .into_iter()
                .filter_map(|stream| {
                    let rel = entry[stream].as_str()?;
                    Some((stream.to_string(), self.dir.join(rel).display().to_string()))
                })
                .collect();
            if let Some(notes) = &self.notes {
                // Bazel's events spell a non-ASCII label as Latin-1.
                let label = crate::bes::repaired(label).unwrap_or_else(|| label.to_string());
                for (stream, path) in &streams {
                    notes.push(format!("INFO: {label} {mnemonic} {stream}: {path}").into_bytes());
                }
            }
        }
        self.actions.push(entry);
    }

    fn test(&mut self, result: &TestResult, id: &TestResultId) {
        let status = TestStatus::try_from(result.status).unwrap_or(TestStatus::NoStatus);
        // As `ArtifactUpload` reads it: a flaky attempt did not pass.
        let passed = status == TestStatus::Passed;
        let cached = attempt_cached(result);
        let wanted = match self.keep.tests {
            TestLogs::None => false,
            TestLogs::Failed => !passed,
            TestLogs::Executed => !cached,
            TestLogs::All => true,
        };
        let config = config_of(&id.configuration);
        let last = self
            .last_attempts
            .entry((id.label.clone(), config.clone(), id.run, id.shard))
            .or_default();
        *last = (*last).max(id.attempt);
        if !wanted || result.test_action_output.is_empty() {
            return;
        }
        let dir = Path::new("testlogs")
            .join(label_dir(&id.label))
            .join(format!("run_{}", id.run))
            .join(format!("shard_{}", id.shard))
            .join(format!("attempt_{}", id.attempt));
        let mut files = serde_json::Map::new();
        for file in &result.test_action_output {
            let name = if file.name.is_empty() {
                "output"
            } else {
                file.name.as_str()
            };
            let rel = dir.join(sanitize_path(name));
            if is_remote(file) && matches!(name, "test.log" | "test.xml") {
                self.remote_test_logs.push(RemoteTestLog {
                    entry: self.tests.len(),
                    name: name.to_string(),
                    rel,
                    label: id.label.clone(),
                    config: config.clone(),
                    run: id.run,
                    shard: id.shard,
                    attempt: id.attempt,
                });
                continue;
            }
            if let Some(kept) = keep_file(&self.dir, file, &rel, &mut self.tests_not_kept) {
                files.insert(name.to_string(), json!(kept));
            }
        }
        let mut entry = json!({
            "label": id.label,
            "run": id.run,
            "shard": id.shard,
            "attempt": id.attempt,
            "status": status.as_str_name().to_ascii_lowercase(),
            "cached": cached,
            "files": files,
        });
        #[allow(deprecated)]
        let legacy_ms = result.test_attempt_start_millis_epoch;
        if let Some(start) = result.test_attempt_start.as_ref() {
            entry["start"] = json!(timestamp(start));
        } else if legacy_ms > 0 {
            entry["start"] = json!(rfc3339(
                legacy_ms.div_euclid(1000),
                (legacy_ms.rem_euclid(1000) * 1_000_000) as u32
            ));
        }
        self.tests.push(entry);
    }

    /// Keep the test logs a remote URI named from where Bazel left them, with
    /// the run and shard counts and last attempts known so far (unknown
    /// counts resolve only the first run and shard).
    fn find_remote_test_logs(&mut self) {
        for log in std::mem::take(&mut self.remote_test_logs) {
            let Some(source) = self.local_test_log(&log).filter(|p| p.is_file()) else {
                self.tests_not_kept.not_local += 1;
                continue;
            };
            match place(&Source::Path(source.clone()), &self.dir, &log.rel) {
                Ok(kept) => {
                    self.tests[log.entry]["files"][log.name.as_str()] =
                        json!(kept.to_string_lossy());
                }
                Err(e) => {
                    tracing::debug!("could not keep {}: {e}", source.display());
                    self.tests_not_kept.failed += 1;
                }
            }
        }
    }

    /// Where Bazel leaves a test's log locally, as its output tree lays them
    /// out: `<exec root>/bazel-out/<output dir>/testlogs/[external/<repo>/]
    /// <package>/<name>/[shard_<s>_of_<n>][_][run_<r>_of_<m>]/`,
    /// `test.<ext>` for a run and shard's last attempt, else
    /// `test_attempts/attempt_<a>.<ext>`.
    fn local_test_log(&self, log: &RemoteTestLog) -> Option<PathBuf> {
        let (repo, rest) = log.label.trim_start_matches('@').split_once("//")?;
        let (package, name) = rest.split_once(':')?;
        // Unknown counts (no summary yet) resolve only for the first run and
        // shard, as `testlog_path` in `bazel_results.axl` has it.
        let (runs, shards) = self
            .test_counts
            .get(&(log.label.clone(), log.config.clone()))
            .copied()
            .unwrap_or((0, 0));
        if (shards == 0 && log.shard > 1) || (runs == 0 && log.run > 1) {
            return None;
        }
        let mut dir = self
            .exec_root
            .as_ref()?
            .join("bazel-out")
            .join(self.output_dirs.get(&log.config)?)
            .join("testlogs");
        if !repo.is_empty() {
            dir.push("external");
            dir.push(repo);
        }
        for part in package.split('/').chain(name.split('/')) {
            if !part.is_empty() {
                dir.push(part);
            }
        }
        let mut sub = vec![];
        if shards > 1 {
            sub.push(format!("shard_{}_of_{shards}", log.shard));
        }
        if runs > 1 {
            sub.push(format!("run_{}_of_{runs}", log.run));
        }
        if !sub.is_empty() {
            dir.push(sub.join("_"));
        }
        let ext = log.name.strip_prefix("test.")?;
        let last =
            self.last_attempts
                .get(&(log.label.clone(), log.config.clone(), log.run, log.shard));
        Some(if last.is_some_and(|&a| log.attempt < a) {
            dir.join("test_attempts")
                .join(format!("attempt_{}.{ext}", log.attempt))
        } else {
            dir.join(format!("test.{ext}"))
        })
    }

    /// Write each kind's index, for what was asked to be kept.
    fn finish(mut self) {
        self.find_remote_test_logs();
        if self.keep.actions != ActionLogs::None {
            write_index(
                &self.dir.join("actions.json"),
                json!({ "actions": self.actions, "not_kept": self.actions_not_kept.json() }),
            );
        }
        if self.keep.tests != TestLogs::None {
            write_index(
                &self.dir.join("testlogs.json"),
                json!({ "tests": self.tests, "not_kept": self.tests_not_kept.json() }),
            );
        }
    }
}

/// Keep `file` at `rel` under `dir`; the relative path, or None when it was
/// remote, inline or could not be kept (counted in `not_kept`).
fn keep_file(dir: &Path, file: &File, rel: &Path, not_kept: &mut NotKept) -> Option<String> {
    let source = match (
        crate::bes::uri_of(file).and_then(crate::bes::local_path),
        file.file.as_ref(),
    ) {
        (Some(path), _) => Source::Path(path),
        (None, Some(file::File::Contents(bytes))) => Source::Bytes(bytes),
        _ => {
            not_kept.not_local += 1;
            return None;
        }
    };
    match place(&source, dir, rel) {
        Ok(kept) => Some(kept.to_string_lossy().into_owned()),
        Err(e) => {
            tracing::debug!("could not keep {}: {e}", source.describe());
            not_kept.failed += 1;
            None
        }
    }
}

/// What a log is kept from: a local file, or the contents Bazel sent inline.
enum Source<'a> {
    Path(PathBuf),
    Bytes(&'a [u8]),
}

impl Source<'_> {
    fn describe(&self) -> String {
        match self {
            Source::Path(path) => path.display().to_string(),
            Source::Bytes(bytes) => format!("{} inline bytes", bytes.len()),
        }
    }
}

/// Whether `file` is named only by a URI that is not local (`bytestream://`).
fn is_remote(file: &File) -> bool {
    matches!(file.file.as_ref(), Some(file::File::Uri(uri)) if !uri.starts_with("file://"))
}

/// A configuration id, `""` for none.
fn config_of(id: &Option<ConfigurationId>) -> String {
    id.as_ref().map(|c| c.id.clone()).unwrap_or_default()
}

/// Put `from` at `rel` under `dir`, making its folders: a file hard-linked,
/// else copied, inline contents written; the path it took, relative to
/// `dir`. Nothing is overwritten: two labels can map to one folder
/// (`//a:b/c`, `//a/b:c`), so a taken name gets a numbered sibling
/// (`test.log` → `test.2.log`).
fn place(from: &Source, dir: &Path, rel: &Path) -> io::Result<PathBuf> {
    if let Some(parent) = dir.join(rel).parent() {
        fs::create_dir_all(parent)?;
    }
    for n in 1..=MAX_SIBLINGS {
        let candidate = if n == 1 {
            rel.to_path_buf()
        } else {
            numbered(rel, n)
        };
        let to = dir.join(&candidate);
        if let Source::Path(from) = from {
            match fs::hard_link(from, &to) {
                Ok(()) => return Ok(candidate),
                Err(e) if e.kind() == io::ErrorKind::AlreadyExists => continue,
                Err(_) => {}
            }
        }
        // A copy that never replaces what is there, nor leaves a partial or
        // empty file where a log looks kept.
        let mut source: Box<dyn io::Read> = match from {
            Source::Path(from) => Box::new(fs::File::open(from)?),
            Source::Bytes(bytes) => Box::new(*bytes),
        };
        let mut out = match fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&to)
        {
            Ok(out) => out,
            Err(e) if e.kind() == io::ErrorKind::AlreadyExists => continue,
            Err(e) => return Err(e),
        };
        if let Err(e) = io::copy(&mut source, &mut out) {
            drop(out);
            let _ = fs::remove_file(&to);
            return Err(e);
        }
        return Ok(candidate);
    }
    Err(io::Error::new(
        io::ErrorKind::AlreadyExists,
        "too many logs of one name",
    ))
}

/// How many logs of one name a folder takes before the next is not kept.
const MAX_SIBLINGS: usize = 100;

/// `rel` with `.<n>` before its extension: `a/test.log` → `a/test.2.log`.
fn numbered(rel: &Path, n: usize) -> PathBuf {
    let stem = rel
        .file_stem()
        .map(|s| s.to_string_lossy().into_owned())
        .unwrap_or_default();
    let name = match rel.extension() {
        Some(ext) => format!("{stem}.{n}.{}", ext.to_string_lossy()),
        None => format!("{stem}.{n}"),
    };
    rel.with_file_name(name)
}

fn write_index(path: &Path, value: Value) {
    let write = || -> io::Result<()> {
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)?;
        }
        fs::write(path, serde_json::to_vec_pretty(&value)?)
    };
    if let Err(e) = write() {
        tracing::debug!("could not write {}: {e}", path.display());
    }
}

/// The folder a label's logs go in: its repository, package and name, each
/// a path component that cannot climb out (`//a/b:c` → `a/b/c`,
/// `@repo//a/b:c` → `external/repo/a/b/c`), a non-ASCII name as meant
/// (`crate::bes::repaired`), not as Bazel's events spell it.
fn label_dir(label: &str) -> PathBuf {
    let meant = crate::bes::repaired(label);
    let label = meant.as_deref().unwrap_or(label).trim_start_matches('@');
    let (repo, rest) = match label.split_once("//") {
        Some((repo, rest)) => (repo, rest),
        None => ("", label),
    };
    let (package, name) = rest.split_once(':').unwrap_or((rest, ""));
    // An external repository's under `external/`, as in Bazel's output tree,
    // so `@foo//x:t` and `//foo/x:t` do not share a folder.
    let mut dir = PathBuf::new();
    if !repo.is_empty() {
        dir.push("external");
        dir.push(sanitize(repo));
    }
    for part in package.split('/').filter(|p| !p.is_empty()) {
        dir.push(sanitize(part));
    }
    for part in name.split('/').filter(|p| !p.is_empty()) {
        dir.push(sanitize(part));
    }
    if dir.as_os_str().is_empty() {
        dir.push("_");
    }
    dir
}

/// `name` as one path component: no separators, and never `.` or `..`.
fn sanitize(name: &str) -> String {
    let clean: String = name
        .chars()
        .map(|c| {
            if c == '/' || c == '\\' || c.is_control() {
                '_'
            } else {
                c
            }
        })
        .collect();
    match clean.as_str() {
        "" | "." | ".." => "_".to_string(),
        _ => clean,
    }
}

/// `name`, a relative path a test output names (`test.outputs/x`), with every
/// component made safe ([`sanitize`]).
fn sanitize_path(name: &str) -> PathBuf {
    name.split('/')
        .filter(|p| !p.is_empty())
        .map(sanitize)
        .collect()
}

/// A protobuf timestamp as RFC 3339 UTC, to the millisecond.
fn timestamp(t: &prost_types::Timestamp) -> String {
    rfc3339(t.seconds, t.nanos.max(0) as u32)
}

/// `secs` and `nanos` since the epoch as `YYYY-MM-DDTHH:MM:SS.mmmZ`.
fn rfc3339(secs: i64, nanos: u32) -> String {
    chrono::DateTime::<chrono::Utc>::from_timestamp(secs, nanos)
        .map(|t| t.to_rfc3339_opts(chrono::SecondsFormat::Millis, true))
        .unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;
    use axl_proto::build_event_stream::test_result::ExecutionInfo;
    use axl_proto::build_event_stream::{BuildEvent, BuildEventId};

    fn uri_file(name: &str, path: &Path) -> File {
        File {
            name: name.into(),
            file: Some(file::File::Uri(format!("file://{}", path.display()))),
            ..Default::default()
        }
    }

    fn action_event(
        label: &str,
        mnemonic: &str,
        success: bool,
        stdout: Option<File>,
        stderr: Option<File>,
    ) -> BuildEvent {
        BuildEvent {
            id: Some(BuildEventId {
                id: Some(Id::ActionCompleted(ActionCompletedId {
                    primary_output: format!("bazel-out/bin/{mnemonic}.out"),
                    label: label.into(),
                    configuration: None,
                })),
            }),
            payload: Some(Payload::Action(ActionExecuted {
                success,
                r#type: mnemonic.into(),
                exit_code: if success { 0 } else { 1 },
                stdout,
                stderr,
                ..Default::default()
            })),
            ..Default::default()
        }
    }

    fn test_event(label: &str, status: TestStatus, cached: bool, files: Vec<File>) -> BuildEvent {
        BuildEvent {
            id: Some(BuildEventId {
                id: Some(Id::TestResult(TestResultId {
                    label: label.into(),
                    run: 1,
                    shard: 1,
                    attempt: 1,
                    configuration: None,
                })),
            }),
            payload: Some(Payload::TestResult(TestResult {
                status: status as i32,
                cached_locally: cached,
                test_action_output: files,
                execution_info: Some(ExecutionInfo::default()),
                ..Default::default()
            })),
            ..Default::default()
        }
    }

    fn read_index(path: &Path) -> Value {
        serde_json::from_slice(&fs::read(path).unwrap()).unwrap()
    }

    /// A failed action's kept log is told to the console as an `INFO:`
    /// line naming its label, mnemonic, stream and path; a successful
    /// one's (`all`) is not.
    #[test]
    fn a_failed_actions_kept_log_is_noted_for_the_console() {
        let tmp = tempfile::tempdir().unwrap();
        let src = tmp.path().join("src");
        fs::create_dir_all(&src).unwrap();
        fs::write(src.join("stderr-1"), "ERROR: broken\n").unwrap();
        fs::write(src.join("stdout-2"), "ok\n").unwrap();
        let dir = tmp.path().join("history");
        let mut kept = Kept::new(
            Keep {
                actions: ActionLogs::All,
                tests: TestLogs::None,
            },
            dir.clone(),
        );
        let notes = crate::console::forward::Notes::default();
        kept.notes = Some(notes.clone());
        kept.event(&action_event(
            "//pkg:broken",
            "Genrule",
            false,
            None,
            Some(uri_file("stderr", &src.join("stderr-1"))),
        ));
        kept.event(&action_event(
            "//pkg:fine",
            "Genrule",
            true,
            Some(uri_file("stdout", &src.join("stdout-2"))),
            None,
        ));
        let got: Vec<String> = notes
            .take()
            .into_iter()
            .map(|n| String::from_utf8(n).unwrap())
            .collect();
        assert_eq!(got.len(), 1, "{got:?}");
        assert!(
            got[0].starts_with("INFO: //pkg:broken Genrule stderr: "),
            "{got:?}"
        );
        assert!(got[0].ends_with("Genrule-1.stderr"), "{got:?}");
    }

    /// The note spells a label as meant, not as Bazel's events do (a
    /// non-ASCII label's UTF-8 bytes read as Latin-1).
    #[test]
    fn a_kept_logs_note_repairs_its_label() {
        let tmp = tempfile::tempdir().unwrap();
        let src = tmp.path().join("src");
        fs::create_dir_all(&src).unwrap();
        fs::write(src.join("stderr-1"), "broke\n").unwrap();
        let mut kept = Kept::new(
            Keep {
                actions: ActionLogs::Failed,
                tests: TestLogs::None,
            },
            tmp.path().join("history"),
        );
        let notes = crate::console::forward::Notes::default();
        kept.notes = Some(notes.clone());
        let latin1: String = "//uni:café".bytes().map(char::from).collect();
        kept.event(&action_event(
            &latin1,
            "Genrule",
            false,
            None,
            Some(uri_file("stderr", &src.join("stderr-1"))),
        ));
        let got: Vec<String> = notes
            .take()
            .into_iter()
            .map(|n| String::from_utf8(n).unwrap())
            .collect();
        assert!(
            got[0].starts_with("INFO: //uni:café Genrule stderr: "),
            "{got:?}"
        );
    }

    /// Each kept action is published by its primary output as it is kept,
    /// before the index is written, at the path it was kept at.
    #[test]
    fn kept_actions_are_published_as_they_are_kept() {
        let tmp = tempfile::tempdir().unwrap();
        let src = tmp.path().join("src");
        fs::create_dir_all(&src).unwrap();
        fs::write(src.join("stderr-1"), "ERROR: broken\n").unwrap();
        let dir = tmp.path().join("history");
        let mut kept = Kept::new(
            Keep {
                actions: ActionLogs::Failed,
                tests: TestLogs::None,
            },
            dir.clone(),
        );
        let published = KeptActions::default();
        kept.published = Some(published.clone());
        kept.event(&action_event(
            "//pkg:broken",
            "Genrule",
            false,
            None,
            Some(uri_file("stderr", &src.join("stderr-1"))),
        ));
        let got = published
            .lock()
            .unwrap()
            .get("bazel-out/bin/Genrule.out")
            .cloned()
            .unwrap();
        assert!(got.stdout.is_none());
        let stderr = got.stderr.unwrap();
        assert!(stderr.starts_with(&dir), "{stderr:?}");
        // Bazel replacing its own file leaves the kept one as it was.
        fs::remove_file(src.join("stderr-1")).unwrap();
        assert_eq!(fs::read_to_string(&stderr).unwrap(), "ERROR: broken\n");
        assert!(
            !dir.join("actions.json").exists(),
            "published before the index"
        );
    }

    #[test]
    fn failed_actions_are_linked_and_indexed() {
        let tmp = tempfile::tempdir().unwrap();
        let src = tmp.path().join("src");
        fs::create_dir_all(&src).unwrap();
        fs::write(src.join("stderr-2"), "ERROR: broken\n").unwrap();
        fs::write(src.join("stdout-3"), "ok\n").unwrap();
        let dir = tmp.path().join("history");
        let mut kept = Kept::new(
            Keep {
                actions: ActionLogs::Failed,
                tests: TestLogs::None,
            },
            dir.clone(),
        );
        kept.event(&action_event(
            "//pkg:broken",
            "Genrule",
            false,
            None,
            Some(uri_file("stderr", &src.join("stderr-2"))),
        ));
        kept.event(&action_event(
            "//pkg:fine",
            "Genrule",
            true,
            Some(uri_file("stdout", &src.join("stdout-3"))),
            None,
        ));
        kept.finish();
        let kept_file = dir.join("actions/pkg/broken/Genrule-1.stderr");
        assert_eq!(fs::read_to_string(&kept_file).unwrap(), "ERROR: broken\n");
        #[cfg(unix)]
        {
            use std::os::unix::fs::MetadataExt;
            assert_eq!(
                fs::metadata(&kept_file).unwrap().ino(),
                fs::metadata(src.join("stderr-2")).unwrap().ino(),
                "hard-linked"
            );
        }
        assert!(
            !dir.join("actions/pkg/fine").exists(),
            "a success is not kept under `failed`"
        );
        let index = read_index(&dir.join("actions.json"));
        assert_eq!(index["actions"].as_array().unwrap().len(), 1);
        assert_eq!(
            index["actions"][0]["stderr"],
            "actions/pkg/broken/Genrule-1.stderr"
        );
        assert_eq!(index["actions"][0]["exit_code"], 1);
        assert!(!dir.join("testlogs").exists(), "test logs not asked for");
    }

    #[test]
    fn all_keeps_every_action_numbered_per_mnemonic_and_counts_remote_logs() {
        let tmp = tempfile::tempdir().unwrap();
        let src = tmp.path().join("out");
        fs::write(&src, "x\n").unwrap();
        let dir = tmp.path().join("history");
        let mut kept = Kept::new(
            Keep {
                actions: ActionLogs::All,
                tests: TestLogs::All,
            },
            dir.clone(),
        );
        kept.event(&action_event(
            "@@repo+//a/b:c",
            "CppCompile",
            true,
            Some(uri_file("stdout", &src)),
            None,
        ));
        kept.event(&action_event(
            "@@repo+//a/b:c",
            "CppCompile",
            true,
            Some(uri_file("stdout", &src)),
            None,
        ));
        let remote = File {
            name: "stderr".into(),
            file: Some(file::File::Uri("bytestream://cache/blobs/abc/1".into())),
            ..Default::default()
        };
        kept.event(&action_event(
            "//r:remote",
            "Genrule",
            false,
            None,
            Some(remote),
        ));
        kept.finish();
        assert!(
            dir.join("actions/external/repo+/a/b/c/CppCompile-1.stdout")
                .exists()
        );
        assert!(
            dir.join("actions/external/repo+/a/b/c/CppCompile-2.stdout")
                .exists()
        );
        let index = read_index(&dir.join("actions.json"));
        assert_eq!(index["not_kept"]["not_local"], 1);
        let tests = read_index(&dir.join("testlogs.json"));
        assert_eq!(
            tests["not_kept"]["not_local"], 0,
            "each kind counts its own"
        );
        assert_eq!(
            index["actions"].as_array().unwrap().len(),
            3,
            "a remote-only action is still listed"
        );
    }

    #[test]
    fn test_logs_are_linked_by_status_and_cache() {
        let tmp = tempfile::tempdir().unwrap();
        let log = tmp.path().join("test.log");
        fs::write(&log, "FAIL\n").unwrap();
        let files = || {
            vec![
                uri_file("test.log", &log),
                uri_file("test.outputs/../../escape", &log),
            ]
        };
        for (which, kept_labels) in [
            (TestLogs::Failed, vec!["fails"]),
            (TestLogs::Executed, vec!["fails", "passes"]),
            (TestLogs::All, vec!["fails", "passes", "cached"]),
        ] {
            let dir = tmp.path().join(format!("h-{which:?}"));
            let mut kept = Kept::new(
                Keep {
                    actions: ActionLogs::None,
                    tests: which,
                },
                dir.clone(),
            );
            kept.event(&test_event("//t:fails", TestStatus::Failed, false, files()));
            kept.event(&test_event(
                "//t:passes",
                TestStatus::Passed,
                false,
                files(),
            ));
            kept.event(&test_event("//t:cached", TestStatus::Passed, true, files()));
            kept.finish();
            let index = read_index(&dir.join("testlogs.json"));
            let labels: Vec<String> = index["tests"]
                .as_array()
                .unwrap()
                .iter()
                .map(|t| {
                    t["label"]
                        .as_str()
                        .unwrap()
                        .trim_start_matches("//t:")
                        .to_string()
                })
                .collect();
            assert_eq!(labels, kept_labels, "{which:?}");
            let kept = dir.join("testlogs/t/fails/run_1/shard_1/attempt_1/test.log");
            assert_eq!(fs::read_to_string(&kept).unwrap(), "FAIL\n");
            #[cfg(unix)]
            {
                use std::os::unix::fs::MetadataExt;
                assert_eq!(
                    fs::metadata(&kept).unwrap().ino(),
                    fs::metadata(&log).unwrap().ino(),
                    "hard-linked, as an action's"
                );
            }
            assert!(
                dir.join("testlogs/t/fails/run_1/shard_1/attempt_1/test.outputs/_/_/escape")
                    .exists(),
                "a relative name cannot climb out"
            );
        }
    }

    /// Two labels that map to one folder keep both logs.
    #[test]
    fn a_taken_name_gets_a_numbered_sibling() {
        let tmp = tempfile::tempdir().unwrap();
        let (a, b) = (tmp.path().join("a"), tmp.path().join("b"));
        fs::write(&a, "A\n").unwrap();
        fs::write(&b, "B\n").unwrap();
        let dir = tmp.path().join("history");
        let mut kept = Kept::new(
            Keep {
                actions: ActionLogs::None,
                tests: TestLogs::All,
            },
            dir.clone(),
        );
        kept.event(&test_event(
            "//a:b/c",
            TestStatus::Passed,
            false,
            vec![uri_file("test.log", &a)],
        ));
        kept.event(&test_event(
            "//a/b:c",
            TestStatus::Passed,
            false,
            vec![uri_file("test.log", &b)],
        ));
        kept.finish();
        let base = dir.join("testlogs/a/b/c/run_1/shard_1/attempt_1");
        assert_eq!(fs::read_to_string(base.join("test.log")).unwrap(), "A\n");
        assert_eq!(fs::read_to_string(base.join("test.2.log")).unwrap(), "B\n");
        let index = read_index(&dir.join("testlogs.json"));
        assert_ne!(
            index["tests"][0]["files"]["test.log"],
            index["tests"][1]["files"]["test.log"]
        );
    }

    /// A target named like an index is kept inside its kind's folder, and
    /// the index beside it is whole.
    #[test]
    fn a_target_named_like_an_index_stays_in_its_folder() {
        let tmp = tempfile::tempdir().unwrap();
        let log = tmp.path().join("stderr-1");
        fs::write(&log, "oops\n").unwrap();
        let dir = tmp.path().join("history");
        let mut kept = Kept::new(
            Keep {
                actions: ActionLogs::Failed,
                tests: TestLogs::None,
            },
            dir.clone(),
        );
        kept.event(&action_event(
            "//:actions.json",
            "Genrule",
            false,
            None,
            Some(uri_file("stderr", &log)),
        ));
        kept.finish();
        assert!(dir.join("actions/actions.json/Genrule-1.stderr").is_file());
        assert_eq!(
            read_index(&dir.join("actions.json"))["actions"]
                .as_array()
                .unwrap()
                .len(),
            1
        );
    }

    /// A test log a remote cache names by `bytestream://` is kept from
    /// where Bazel left it: the last attempt's `test.log`, an earlier one's
    /// under `test_attempts/`, a sharded test's in its shard's folder.
    #[test]
    fn a_remotely_named_test_log_is_found_on_disk() {
        use axl_proto::build_event_stream::build_event_id::{ConfigurationId, TestSummaryId};
        use axl_proto::build_event_stream::{Configuration, TestSummary, WorkspaceConfig};
        let tmp = tempfile::tempdir().unwrap();
        let exec_root = tmp.path().join("execroot");
        let logs = exec_root.join("bazel-out/k8-fastbuild/testlogs/pkg");
        let flaky = logs.join("flaky");
        fs::create_dir_all(flaky.join("test_attempts")).unwrap();
        fs::write(flaky.join("test_attempts/attempt_1.log"), "first\n").unwrap();
        fs::write(flaky.join("test.log"), "second\n").unwrap();
        let sharded = logs.join("sharded/shard_2_of_2");
        fs::create_dir_all(&sharded).unwrap();
        fs::write(sharded.join("test.log"), "shard two\n").unwrap();
        let remote = |name: &str| File {
            name: name.into(),
            file: Some(file::File::Uri("bytestream://cache/blobs/abc/1".into())),
            ..Default::default()
        };
        let config = Some(ConfigurationId { id: "c1".into() });
        let result = |label: &str, shard: i32, attempt: i32, status: TestStatus| {
            let mut e = test_event(label, status, false, vec![remote("test.log")]);
            if let Some(Id::TestResult(id)) = e.id.as_mut().and_then(|i| i.id.as_mut()) {
                id.shard = shard;
                id.attempt = attempt;
                id.configuration = config.clone();
            }
            e
        };
        let summary = |label: &str, shards: i32| BuildEvent {
            id: Some(BuildEventId {
                id: Some(Id::TestSummary(TestSummaryId {
                    label: label.into(),
                    configuration: config.clone(),
                })),
            }),
            payload: Some(Payload::TestSummary(TestSummary {
                run_count: 1,
                shard_count: shards,
                ..Default::default()
            })),
            ..Default::default()
        };
        let dir = tmp.path().join("history");
        let mut kept = Kept::new(
            Keep {
                actions: ActionLogs::None,
                tests: TestLogs::All,
            },
            dir.clone(),
        );
        kept.event(&BuildEvent {
            payload: Some(Payload::WorkspaceInfo(WorkspaceConfig {
                local_exec_root: exec_root.display().to_string(),
            })),
            ..Default::default()
        });
        kept.event(&BuildEvent {
            id: Some(BuildEventId {
                id: Some(Id::Configuration(ConfigurationId { id: "c1".into() })),
            }),
            payload: Some(Payload::Configuration(Configuration {
                mnemonic: "k8-fastbuild".into(),
                ..Default::default()
            })),
            ..Default::default()
        });
        kept.event(&result("//pkg:flaky", 1, 1, TestStatus::Failed));
        kept.event(&result("//pkg:flaky", 1, 2, TestStatus::Passed));
        kept.event(&summary("//pkg:flaky", 1));
        kept.event(&result("//pkg:sharded", 2, 1, TestStatus::Passed));
        kept.event(&result("//pkg:gone", 1, 1, TestStatus::Passed));
        kept.event(&summary("//pkg:sharded", 2));
        kept.event(&summary("//pkg:gone", 1));
        kept.finish();
        let read = |rel: &str| fs::read_to_string(dir.join(rel)).unwrap();
        assert_eq!(
            read("testlogs/pkg/flaky/run_1/shard_1/attempt_1/test.log"),
            "first\n"
        );
        assert_eq!(
            read("testlogs/pkg/flaky/run_1/shard_1/attempt_2/test.log"),
            "second\n"
        );
        assert_eq!(
            read("testlogs/pkg/sharded/run_1/shard_2/attempt_1/test.log"),
            "shard two\n"
        );
        let index = read_index(&dir.join("testlogs.json"));
        assert_eq!(
            index["tests"][1]["files"]["test.log"],
            "testlogs/pkg/flaky/run_1/shard_1/attempt_2/test.log"
        );
        assert_eq!(index["not_kept"]["not_local"], 1, "a log on no disk at all");
    }

    fn remote_log(label: &str, run: i32, shard: i32, attempt: i32) -> RemoteTestLog {
        RemoteTestLog {
            entry: 0,
            name: "test.log".into(),
            rel: PathBuf::from("x"),
            label: label.into(),
            config: "c1".into(),
            run,
            shard,
            attempt,
        }
    }

    /// The local paths of test logs, as Bazel's output tree lays them out.
    #[test]
    fn local_test_logs_follow_the_output_tree() {
        let mut kept = Kept::new(
            Keep {
                actions: ActionLogs::None,
                tests: TestLogs::All,
            },
            PathBuf::from("h"),
        );
        kept.exec_root = Some(PathBuf::from("/x"));
        kept.output_dirs.insert("c1".into(), "k8-opt".into());
        let at = |kept: &Kept, log| kept.local_test_log(&log).map(|p| p.display().to_string());
        assert_eq!(
            at(&kept, remote_log("@@//a/b:t", 1, 1, 1)).as_deref(),
            Some("/x/bazel-out/k8-opt/testlogs/a/b/t/test.log"),
            "unknown counts resolve for the first run and shard"
        );
        assert_eq!(
            at(&kept, remote_log("//a:t", 1, 2, 1)),
            None,
            "but not for another shard"
        );
        assert_eq!(
            at(&kept, remote_log("@@rules_x+//p:t", 1, 1, 1)).as_deref(),
            Some("/x/bazel-out/k8-opt/testlogs/external/rules_x+/p/t/test.log")
        );
        kept.test_counts
            .insert(("//a:t".into(), "c1".into()), (3, 2));
        kept.last_attempts
            .insert(("//a:t".into(), "c1".into(), 2, 2), 2);
        assert_eq!(
            at(&kept, remote_log("//a:t", 2, 2, 1)).as_deref(),
            Some(
                "/x/bazel-out/k8-opt/testlogs/a/t/shard_2_of_2_run_2_of_3/test_attempts/attempt_1.log"
            )
        );
        assert_eq!(
            at(&kept, remote_log("//a:t", 2, 2, 2)).as_deref(),
            Some("/x/bazel-out/k8-opt/testlogs/a/t/shard_2_of_2_run_2_of_3/test.log")
        );
    }

    /// A retry of the build in one stream: the abandoned attempt's remote log
    /// is found before the retry replaces it.
    #[test]
    fn a_retried_build_keeps_each_attempts_own_test_log() {
        use axl_proto::build_event_stream::{BuildStarted, Configuration, WorkspaceConfig};
        let tmp = tempfile::tempdir().unwrap();
        let exec_root = tmp.path().join("execroot");
        let logs = exec_root.join("bazel-out/k8/testlogs/pkg/t");
        fs::create_dir_all(&logs).unwrap();
        let dir = tmp.path().join("history");
        let mut kept = Kept::new(
            Keep {
                actions: ActionLogs::None,
                tests: TestLogs::All,
            },
            dir.clone(),
        );
        kept.event(&BuildEvent {
            payload: Some(Payload::WorkspaceInfo(WorkspaceConfig {
                local_exec_root: exec_root.display().to_string(),
            })),
            ..Default::default()
        });
        kept.event(&BuildEvent {
            id: Some(BuildEventId {
                id: Some(Id::Configuration(ConfigurationId { id: "c1".into() })),
            }),
            payload: Some(Payload::Configuration(Configuration {
                mnemonic: "k8".into(),
                ..Default::default()
            })),
            ..Default::default()
        });
        let remote = File {
            name: "test.log".into(),
            file: Some(file::File::Uri("bytestream://cache/blobs/abc/1".into())),
            ..Default::default()
        };
        let mut result = test_event("//pkg:t", TestStatus::Passed, false, vec![remote]);
        if let Some(Id::TestResult(id)) = result.id.as_mut().and_then(|i| i.id.as_mut()) {
            id.configuration = Some(ConfigurationId { id: "c1".into() });
        }
        let started = BuildEvent {
            payload: Some(Payload::Started(BuildStarted::default())),
            ..Default::default()
        };
        kept.event(&started);
        fs::write(logs.join("test.log"), "first\n").unwrap();
        kept.event(&result);
        kept.event(&started);
        // As Bazel runs a test again: its log a new file at the same name.
        fs::remove_file(logs.join("test.log")).unwrap();
        fs::write(logs.join("test.log"), "retry\n").unwrap();
        kept.event(&result);
        kept.finish();
        let index = read_index(&dir.join("testlogs.json"));
        let read = |i: usize| {
            fs::read_to_string(dir.join(index["tests"][i]["files"]["test.log"].as_str().unwrap()))
                .unwrap()
        };
        assert_eq!(
            (read(0), read(1)),
            ("first\n".to_string(), "retry\n".to_string())
        );
    }

    /// The events a [`Keep`] subscribes to: only those its kinds need.
    #[test]
    fn the_filter_passes_only_what_each_kind_needs() {
        use axl_proto::build_event_stream::{BuildStarted, WorkspaceConfig};
        let pass = |keep: Keep, payload: Payload| {
            (keep.filter())(&Arc::new(BuildEventEnvelope {
                encoded: Default::default(),
                event: BuildEvent {
                    payload: Some(payload),
                    ..Default::default()
                },
            }))
        };
        let actions = Keep {
            actions: ActionLogs::All,
            tests: TestLogs::None,
        };
        let tests = Keep {
            actions: ActionLogs::None,
            tests: TestLogs::Failed,
        };
        assert!(pass(actions, Payload::Action(ActionExecuted::default())));
        assert!(!pass(actions, Payload::TestResult(TestResult::default())));
        assert!(!pass(
            actions,
            Payload::WorkspaceInfo(WorkspaceConfig::default())
        ));
        assert!(!pass(tests, Payload::Action(ActionExecuted::default())));
        assert!(pass(tests, Payload::TestResult(TestResult::default())));
        assert!(pass(
            tests,
            Payload::WorkspaceInfo(WorkspaceConfig::default())
        ));
        assert!(pass(tests, Payload::Started(BuildStarted::default())));
        assert!(!pass(tests, Payload::Progress(Default::default())));
    }

    /// Contents Bazel sent inline are written out.
    #[test]
    fn inline_contents_are_kept() {
        let tmp = tempfile::tempdir().unwrap();
        let dir = tmp.path().join("history");
        let inline = File {
            name: "stderr".into(),
            file: Some(file::File::Contents(b"inline\n".to_vec())),
            ..Default::default()
        };
        let mut kept = Kept::new(
            Keep {
                actions: ActionLogs::Failed,
                tests: TestLogs::None,
            },
            dir.clone(),
        );
        kept.event(&action_event(
            "//pkg:broken",
            "Genrule",
            false,
            None,
            Some(inline),
        ));
        kept.finish();
        assert_eq!(
            fs::read_to_string(dir.join("actions/pkg/broken/Genrule-1.stderr")).unwrap(),
            "inline\n"
        );
    }

    /// A source that cannot be opened leaves nothing behind, not an empty log.
    #[test]
    fn a_missing_source_leaves_no_file() {
        let tmp = tempfile::tempdir().unwrap();
        let dir = tmp.path().join("history");
        let err = place(
            &Source::Path(tmp.path().join("gone")),
            &dir,
            Path::new("a/x.stderr"),
        );
        assert!(err.is_err());
        assert!(!dir.join("a/x.stderr").exists());
    }

    #[test]
    fn labels_become_safe_folders() {
        assert_eq!(label_dir("//a/b:c"), PathBuf::from("a/b/c"));
        assert_eq!(label_dir("@@repo+//:x"), PathBuf::from("external/repo+/x"));
        assert_ne!(label_dir("@foo//x:t"), label_dir("//foo/x:t"));
        assert_eq!(label_dir("//:../../etc"), PathBuf::from("_/_/etc"));
        assert_eq!(label_dir(""), PathBuf::from("_"));
        // A non-ASCII label as Bazel's events spell it (UTF-8 as Latin-1).
        let spelled: String = "//uni:тест".bytes().map(char::from).collect();
        assert_eq!(label_dir(&spelled), PathBuf::from("uni/тест"));
    }

    #[test]
    fn file_uris_are_decoded() {
        let f = File {
            file: Some(file::File::Uri("file:///tmp/a%20b/c".into())),
            ..Default::default()
        };
        assert_eq!(
            crate::bes::uri_of(&f).and_then(crate::bes::local_path),
            Some(PathBuf::from("/tmp/a b/c"))
        );
        assert_eq!(rfc3339(0, 5_000_000), "1970-01-01T00:00:00.005Z");
        assert_eq!(rfc3339(1_795_218_795, 0), "2026-11-20T23:53:15.000Z");
    }
}
