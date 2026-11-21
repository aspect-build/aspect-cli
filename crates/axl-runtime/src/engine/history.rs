//! Each task invocation's files, kept for later: one directory per invocation
//! under the history root, named `<start>-<task-id>` where `<start>` is the UTC
//! start time as `YYYYMMDDTHHMMSSmmmZ`, so the names alone list the history in
//! order. A task ID names one run, so an explicit one already here is
//! refused (see [`History::earlier_run`]). Every task's entry is made when it
//! starts, holding its record, `task.json` (see [`task_record`]): what ran,
//! where, how it ended, and each of its Bazel invocations, with a summary of
//! each one's build event stream. A subfolder per Bazel invocation that keeps
//! a file (`bazel/<n>`, counting only those) is made at its spawn.
//!
//! A Bazel invocation's folder holds its compact execution log, workspace log,
//! trace profile and build event stream, each when the build has one, and the
//! action and test logs the build was asked to keep (`actions/` and
//! `testlogs/`, listed in `actions.json` and `testlogs.json`; see `ctx.bazel.build` and
//! `bazel::bes::sink::keep`). The folders are readable by their user alone, since an
//! execution log holds action command lines and environment variables
//! unredacted; the build event stream is kept as the CLI redacted it.
//!
//! Bazel reads some of these files back after the build — it uploads the
//! compact execution log as a build tool log, possibly after the client has
//! exited — so nothing here is moved or deleted by the invocation that wrote
//! it. A [`prune`] removes old invocations' files instead, keeping each one's
//! record with its command line redacted, started in the background by an
//! invocation's first Bazel build that keeps a file, unless another task
//! pruned within
//! [`Retention::every`]: the oldest started first while the history is over
//! [`Retention::budget`], never below [`Retention::keep`] invocations, and
//! never one written to within [`Retention::guard`], which may be a build or an
//! upload still running in another workspace, nor one whose record says its
//! run is still going ([`task_record::still_running`]: another live process,
//! the record written within a week). Past that, a run elsewhere that has
//! gone longer than the guard without writing, and is not among the newest
//! kept, can lose its files; its record, written again later, stays evicted. Entries with
//! only a record (a task that never built, one evicted) count toward neither
//! the budget nor `keep`; they are capped apart, by [`Retention::records`] and
//! [`Retention::record_age`] ([`prune_records`]), also at the end of a run
//! that started no prune. Dotfiles at the root serve the prune: `.pruned`
//! and `.pruned-records`, whose times are the last prune's and the last
//! capping's, and `.sizes`, the sizes of invocations that have settled (and,
//! briefly, its temp file).

use std::collections::HashMap;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::Mutex;
use std::sync::atomic::{AtomicBool, Ordering};
use std::thread::JoinHandle;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use super::task_record::{
    self, BazelRecord, BazelSlot, Link, RecordFile, TaskIdentity, TaskRecord,
};

/// The variable that moves the history root (see [`History::default_root`]).
pub const ROOT_ENV: &str = "ASPECT_HISTORY_DIR";

/// The folder of a task invocation's entry that holds one subfolder per Bazel
/// invocation.
const BAZEL_DIR: &str = "bazel";

/// The folder of Bazel invocation `n` (counted from 1) in the entry at
/// `invocation`.
pub fn bazel_dir(invocation: &Path, n: u32) -> PathBuf {
    invocation.join(BAZEL_DIR).join(n.to_string())
}

/// Create `dir` and its missing parents readable by this user alone: an
/// execution log holds action command lines and environment variables, and
/// nothing redacts it on disk.
pub(crate) fn create_private_dir(dir: &Path) -> io::Result<()> {
    let mut builder = fs::DirBuilder::new();
    builder.recursive(true);
    #[cfg(unix)]
    std::os::unix::fs::DirBuilderExt::mode(&mut builder, 0o700);
    builder.create(dir)
}

/// What the history keeps.
#[derive(Debug, Clone, Copy)]
pub struct Retention {
    /// The size the history is pruned down to.
    pub budget: u64,
    /// Invocations always kept, the most recently started, whatever their size,
    /// besides the current one, which a prune never removes.
    pub keep: usize,
    /// An invocation written to this recently is never pruned.
    pub guard: Duration,
    /// A task skips the prune when any task pruned this recently.
    pub every: Duration,
    /// An invocation idle this long has settled: its size is kept rather than
    /// measured again. Far longer than `guard`, so a build that went quiet and
    /// wrote again is still measured.
    pub settle: Duration,
    /// The most records kept of runs with no files besides their `task.json`
    /// (ones that never built, and ones whose files a prune removed), the
    /// most recently started first.
    pub records: usize,
    /// Such a record older than this goes, by its start.
    pub record_age: Duration,
}

impl Default for Retention {
    fn default() -> Self {
        Self {
            budget: 5 << 30,
            keep: 10,
            guard: Duration::from_secs(60 * 60),
            every: Duration::from_secs(10 * 60),
            settle: Duration::from_secs(24 * 60 * 60),
            records: 5000,
            record_age: Duration::from_secs(90 * 24 * 60 * 60),
        }
    }
}

/// The history root, and the task invocation whose files go into it.
#[derive(Debug)]
pub struct History {
    root: PathBuf,
    retention: Retention,
    current: Mutex<Option<Invocation>>,
    /// See [`History::record_every_build`].
    record: AtomicBool,
    /// See [`History::first_failure`].
    failed: AtomicBool,
    /// The prune started by the first Bazel invocation, left to run beside
    /// the build; see [`prune`].
    pruning: Mutex<Option<JoinHandle<()>>>,
    /// The command line the task was run with; see
    /// [`set_command_line`](Self::set_command_line).
    command_line: Mutex<(Vec<String>, Vec<String>, Vec<String>)>,
    /// The task invocation's `task.json`, once [`begin_task`](Self::begin_task)
    /// has written it.
    task_record: Mutex<Option<Arc<RecordFile>>>,
    /// Its Bazel invocations' places in the record.
    slots: Mutex<Vec<Arc<BazelSlot>>>,
}

/// What a task's record starts with, beside its command line.
#[derive(Debug, Clone, Default)]
pub struct TaskStart {
    pub name: String,
    pub kind: String,
    pub group: Vec<String>,
    pub cli_version: String,
    pub workspace: Option<PathBuf>,
    /// The repository whose commit, branch and state are noted.
    pub git_root: Option<PathBuf>,
}

#[derive(Debug)]
struct Invocation {
    dir: PathBuf,
    bazel_invocations: u32,
}

impl History {
    /// The history at `root`, which need not exist yet.
    pub fn at(root: PathBuf, retention: Retention) -> Self {
        Self {
            root,
            retention,
            current: Mutex::new(None),
            record: AtomicBool::new(false),
            failed: AtomicBool::new(false),
            pruning: Mutex::new(None),
            command_line: Mutex::new((vec![], vec![], vec![])),
            task_record: Mutex::new(None),
            slots: Mutex::new(vec![]),
        }
    }

    /// The command line the task runs with, `argv` as typed (the program
    /// first), the spellings of the flags whose values are secrets, and the
    /// other short flags that take a value, for its record (see
    /// [`task_record`]).
    pub fn set_command_line(
        &self,
        argv: Vec<String>,
        secret_flags: Vec<String>,
        value_shorts: Vec<String>,
    ) {
        *self.command_line.lock().unwrap() = (argv, secret_flags, value_shorts);
    }

    /// [`begin`](Self::begin) the task invocation `task_id`, and make its
    /// entry with its record, `task.json`, in it. An entry that cannot be made
    /// leaves the task without one, quietly: a history is a convenience, and
    /// a task with no build to keep files for should not warn about it.
    pub fn begin_task(&self, task_id: &str, start: TaskStart) {
        self.begin(task_id);
        let dir = match self.current.lock().unwrap().as_ref() {
            Some(invocation) => invocation.dir.clone(),
            None => return,
        };
        let (argv, secret_flags, value_shorts) = self.command_line.lock().unwrap().clone();
        let record = TaskRecord {
            schema: task_record::SCHEMA,
            status: "running".into(),
            cli_version: start.cli_version,
            task: TaskIdentity {
                id: task_id.to_owned(),
                name: start.name,
                kind: start.kind,
                group: start.group,
            },
            pid: std::process::id(),
            argv,
            secret_flags,
            value_shorts,
            cwd: std::env::current_dir()
                .map(|d| d.to_string_lossy().into_owned())
                .unwrap_or_default(),
            workspace: start.workspace.map(|w| w.to_string_lossy().into_owned()),
            started: task_record::timestamp(SystemTime::now()),
            ..Default::default()
        };
        let made =
            create_private_dir(&dir).and_then(|()| RecordFile::begin(&dir, record, start.git_root));
        match made {
            Ok(file) => *self.task_record.lock().unwrap() = Some(file),
            Err(e) => tracing::debug!("no history record for this task in {}: {e}", dir.display()),
        }
    }

    /// Note a Bazel invocation of the task: its command, patterns and flags
    /// (redacted here, the values of the task's secret flags among them), and
    /// its history `folder` when it keeps files. None without a record.
    pub fn bazel_started(
        &self,
        command: &str,
        patterns: &[String],
        flags: &[String],
        folder: Option<&Path>,
    ) -> Option<Arc<BazelSlot>> {
        let file = self.task_record.lock().unwrap().clone()?;
        let entry = file.path().parent()?.to_path_buf();
        // A secret the task passes on into a flag is redacted there too.
        let secrets = self.secret_values();
        let record = BazelRecord {
            command: command.to_owned(),
            patterns: task_record::scrub(patterns, &secrets),
            flags: bazel::bes::redact::redact_command_args(
                task_record::scrub(flags, &secrets)
                    .iter()
                    .map(String::as_str),
            ),
            folder: folder
                .and_then(|f| f.strip_prefix(&entry).ok())
                .map(|f| f.to_string_lossy().into_owned()),
            started: task_record::timestamp(SystemTime::now()),
            ..Default::default()
        };
        let slot = BazelSlot::new(file, record, folder.map(Path::to_path_buf));
        self.slots.lock().unwrap().push(slot.clone());
        Some(slot)
    }

    /// Up to `limit` records that `keep` takes, the most recently started
    /// first, with their entries' folders; unreadable ones skipped.
    pub fn records(
        &self,
        limit: usize,
        keep: impl Fn(&TaskRecord) -> bool,
    ) -> Vec<(PathBuf, TaskRecord)> {
        let Ok(read) = fs::read_dir(&self.root) else {
            return vec![];
        };
        let mut dirs: Vec<PathBuf> = read
            .filter_map(|e| e.ok())
            .filter(|e| is_invocation_name(&e.file_name()))
            .map(|e| e.path())
            .collect();
        dirs.sort();
        dirs.into_iter()
            .rev()
            .filter_map(|dir| {
                let record = task_record::read(&dir.join(task_record::FILE_NAME))?;
                Some((dir, record))
            })
            .filter(|(_, r)| keep(r))
            .take(limit)
            .collect()
    }

    /// The record of the run with task ID `id`, compared ignoring ASCII
    /// case, or the only one whose ID starts with it, or whose entry's
    /// folder is named `id`.
    pub fn record(&self, id: &str) -> Option<(PathBuf, TaskRecord)> {
        let dir = self
            .earlier_run(id)
            .or_else(|| {
                let dir = self.root.join(id);
                is_invocation_name(id.as_ref()).then_some(dir)
            })
            .or_else(|| self.only_run_starting(id))?;
        let record = task_record::read(&dir.join(task_record::FILE_NAME))?;
        Some((dir, record))
    }

    /// The values the task's secret flags were given on its command line.
    fn secret_values(&self) -> Vec<String> {
        let (argv, secret_flags, value_shorts) = &*self.command_line.lock().unwrap();
        task_record::secret_values(argv, secret_flags, value_shorts)
    }

    /// Add a link (a results page) to the task's record, as the record's
    /// other text is kept: the task's secrets' values scrubbed from it, and
    /// the URL's credentials (`user:pass@`) stripped.
    pub fn link(&self, label: &str, url: &str) {
        if let Some(file) = self.task_record.lock().unwrap().as_ref() {
            let secrets = self.secret_values();
            let url = bazel::bes::redact::strip_url_creds(url);
            let text = task_record::scrub(&[label.to_owned(), url.into_owned()], &secrets);
            file.update(|r| {
                let link = Link {
                    label: text[0].clone(),
                    url: text[1].clone(),
                };
                if !r.links.contains(&link) {
                    r.links.push(link);
                }
            });
        }
    }

    /// The task has ended: its record says how, with the summaries of its
    /// builds whose streams are over (one never waited for may not be).
    pub fn end_task(
        &self,
        exit_code: Option<u8>,
        conclusion: String,
        error: String,
        cancelled: bool,
    ) {
        for slot in self.slots.lock().unwrap().drain(..) {
            slot.summarize(true);
        }
        let Some(file) = self.task_record.lock().unwrap().take() else {
            return;
        };
        file.end(exit_code, conclusion, error, cancelled);
        // A run that started no prune (it kept no build's files, or another
        // task had pruned) still caps the records, which runs like it add,
        // unless one did within `every`: a walk of the root's names, cheap
        // beside a prune.
        let pruned = self.pruning.lock().unwrap().is_some();
        if !pruned
            && let Some(current) = file.path().parent()
            && claim_marker(&self.root, RECORDS_PRUNED_MARKER, self.retention.every)
        {
            prune_records(&self.root, self.retention, current);
        }
    }

    /// Whether this is the first folder the history could not create, so a
    /// history that cannot be written is reported once per run, not per build.
    pub fn first_failure(&self) -> bool {
        !self.failed.swap(true, Ordering::SeqCst)
    }

    /// Have every Bazel build of the task write its compact execution log
    /// here, even with nothing to read it.
    pub fn record_every_build(&self) {
        self.record.store(true, Ordering::SeqCst);
    }

    /// Whether [`record_every_build`](Self::record_every_build) was asked for.
    pub fn records_every_build(&self) -> bool {
        self.record.load(Ordering::SeqCst)
    }

    /// The default root: `$ASPECT_HISTORY_DIR`, else `<cache dir>/aspect/history`,
    /// else `aspect-history` in the temp dir on a machine with no cache dir.
    /// It does not follow `$ASPECT_CLI_CACHE`, which CI points at what its
    /// warming archive keeps: unredacted execution logs stay out of that.
    pub fn default_root() -> PathBuf {
        if let Some(dir) = std::env::var_os(ROOT_ENV).filter(|d| !d.is_empty()) {
            return PathBuf::from(dir);
        }
        dirs::cache_dir()
            .map(|dir| dir.join("aspect").join("history"))
            .unwrap_or_else(|| std::env::temp_dir().join("aspect-history"))
    }

    /// Where the history is kept.
    pub fn root(&self) -> &Path {
        &self.root
    }

    /// The entry an earlier run of `task_id` left here, if it is still kept.
    /// IDs are compared ignoring ASCII case, as a UUID is.
    pub fn earlier_run(&self, task_id: &str) -> Option<PathBuf> {
        fs::read_dir(&self.root)
            .ok()?
            .filter_map(|e| e.ok())
            .find(|e| {
                let name = e.file_name();
                is_invocation_name(&name)
                    && name
                        .to_str()
                        .is_some_and(|n| n[STAMP_LEN + 1..].eq_ignore_ascii_case(task_id))
            })
            .map(|e| e.path())
    }

    /// The one entry whose task ID starts with `prefix`, ignoring ASCII
    /// case; None for none or several.
    fn only_run_starting(&self, prefix: &str) -> Option<PathBuf> {
        if prefix.is_empty() {
            return None;
        }
        let prefix = prefix.to_ascii_lowercase();
        let mut found = fs::read_dir(&self.root)
            .ok()?
            .filter_map(|e| e.ok())
            .filter(|e| {
                let name = e.file_name();
                is_invocation_name(&name)
                    && name.to_str().is_some_and(|n| {
                        n[STAMP_LEN + 1..].to_ascii_lowercase().starts_with(&prefix)
                    })
            })
            .map(|e| e.path());
        let first = found.next()?;
        found.next().is_none().then_some(first)
    }

    /// Start the task invocation `task_id`, now. Nothing is created on disk
    /// until its first Bazel invocation asks for a directory, unless
    /// [`begin_task`](Self::begin_task) makes its record.
    pub fn begin(&self, task_id: &str) {
        let dir = self
            .root
            .join(format!("{}-{task_id}", stamp(SystemTime::now())));
        *self.current.lock().unwrap() = Some(Invocation {
            dir,
            bazel_invocations: 0,
        });
    }

    /// A new directory for the next Bazel invocation of the current task
    /// invocation, `<invocation>/bazel/<n>`, created. The first one starts a
    /// prune of the history in the background, unless another task pruned
    /// within [`Retention::every`]. Without [`begin`](Self::begin) an
    /// invocation is begun under a fresh id.
    pub fn next_bazel_dir(&self) -> io::Result<PathBuf> {
        let mut current = self.current.lock().unwrap();
        if current.is_none() {
            drop(current);
            self.begin(&uuid::Uuid::new_v4().to_string());
            current = self.current.lock().unwrap();
        }
        let invocation = current.as_mut().expect("begun above");
        invocation.bazel_invocations += 1;
        let dir = bazel_dir(&invocation.dir, invocation.bazel_invocations);
        // Started before the folder is made, which a full disk can refuse:
        // pruning is what frees space there.
        if invocation.bazel_invocations == 1 && claim_prune(&self.root, self.retention.every) {
            let (root, retention, current) =
                (self.root.clone(), self.retention, invocation.dir.clone());
            let handle = std::thread::Builder::new()
                .name("history-prune".into())
                .spawn(move || prune(&root, retention, &current))
                .ok();
            *self.pruning.lock().unwrap() = handle;
        }
        create_private_dir(&dir)?;
        Ok(dir)
    }

    /// Wait for the prune [`next_bazel_dir`](Self::next_bazel_dir) started, if
    /// any.
    #[cfg(test)]
    pub(crate) fn wait_for_prune(&self) {
        if let Some(handle) = self.pruning.lock().unwrap().take() {
            handle.join().expect("the prune panicked");
        }
    }
}

/// Marks the last prune of the history, by its modification time.
const PRUNED_MARKER: &str = ".pruned";

/// Marks the last capping of the records by a run that pruned nothing else.
const RECORDS_PRUNED_MARKER: &str = ".pruned-records";

/// The sizes of settled invocations; see [`prune`].
const SIZES_INDEX: &str = ".sizes";

/// Whether this task prunes `root` now: when no task has within `every`, as
/// the marker's time says. A marker ahead of the clock has expired. The marker
/// is touched first, which keeps most tasks started together from each
/// pruning, though two can; a marker that cannot be written prunes anyway, as
/// on a full disk, where pruning is what frees space.
fn claim_prune(root: &Path, every: Duration) -> bool {
    claim_marker(root, PRUNED_MARKER, every)
}

/// [`claim_prune`]'s claim, of the marker `name`.
fn claim_marker(root: &Path, name: &str, every: Duration) -> bool {
    let marker = root.join(name);
    let recent = fs::metadata(&marker)
        .and_then(|m| m.modified())
        .is_ok_and(|t| !idle_for(t, every) && t <= SystemTime::now());
    if recent {
        return false;
    }
    let _ = fs::write(&marker, b"");
    true
}

/// A settled invocation's size, and the newest modification time under it
/// when it was measured.
#[derive(Debug, Clone, Copy, PartialEq)]
struct Settled {
    size: u64,
    newest: u64,
}

/// Remove old invocations' files under `root` as `retention` says, leaving
/// `current` alone; each one's record stays ([`evict`]), and the records are
/// then capped ([`prune_records`]). Only entries holding files count here.
/// Best effort: an entry that cannot be read or removed is skipped.
///
/// It runs beside the build and is not waited for: the CLI may exit midway,
/// which leaves only a valid history: part of an entry it was removing, which
/// a later prune that would remove it finishes, once the removals that make
/// it look recent are older than the guard; or an index's temp file, which a
/// prune removes once it is older than the guard. A time ahead of the clock is
/// recent, so an entry written under a clock since set back is kept until the
/// clock catches up: keeping too much is the safe way to be wrong.
///
/// An invocation nothing has written to within [`Retention::settle`] has
/// settled: its size is kept in the [`SIZES_INDEX`] and not measured again
/// while its `bazel` folder, which a further Bazel invocation of it updates,
/// is no newer. So a prune walks only new and live entries, and stats each
/// settled one. An entry the budget would remove is measured first, so one
/// written to since it settled is seen to be live and its own size is never
/// taken on the index's word; the sizes of the newer entries counted before
/// it are, so a file removed by hand from a settled entry leaves the total
/// high until the entry is measured again. The index is a cache, rebuilt whenever it is missing or
/// unreadable, and a prune running elsewhere at once costs at most a walk.
fn prune(root: &Path, retention: Retention, current: &Path) {
    let Ok(read) = fs::read_dir(root) else {
        return;
    };
    let mut entries: Vec<PathBuf> = read
        .filter_map(|e| e.ok())
        .filter(|e| e.file_type().is_ok_and(|t| t.is_dir()))
        .filter(|e| is_invocation_name(&e.file_name()))
        .map(|e| e.path())
        .filter(|p| p != current && holds_files(p))
        .collect();
    // Names start with the start time, so this is the most recent first.
    entries.sort();
    entries.reverse();
    // An index write cut off before its rename leaves its temp file behind.
    for e in fs::read_dir(root)
        .into_iter()
        .flatten()
        .filter_map(|e| e.ok())
    {
        let name = e.file_name();
        let name = name.to_string_lossy();
        let stale = e
            .metadata()
            .and_then(|m| m.modified())
            .is_ok_and(|t| idle_for(t, retention.guard));
        if name.starts_with(&format!("{SIZES_INDEX}.")) && name.ends_with(".tmp") && stale {
            let _ = fs::remove_file(e.path());
        }
    }
    let index_path = root.join(SIZES_INDEX);
    let index = read_index(&index_path);
    let mut settled = HashMap::new();
    let within_guard = |t: SystemTime| !idle_for(t, retention.guard);
    let mut total = 0u64;
    for (i, entry) in entries.iter().enumerate() {
        let name = entry.file_name().unwrap().to_string_lossy().into_owned();
        let known = index.get(&name).copied().filter(|s| {
            fs::metadata(entry.join(BAZEL_DIR))
                .and_then(|m| m.modified())
                .is_ok_and(|t| secs(t) <= s.newest)
        });
        let measure = || {
            let (size, newest) = tree_size_and_newest(entry);
            let settles = newest.is_some_and(|t| idle_for(t, retention.settle));
            (
                size,
                newest.is_some_and(within_guard),
                settles.then(|| Settled {
                    size,
                    newest: secs(newest.unwrap()),
                }),
            )
        };
        let (mut size, mut recent, mut settles) = match known {
            Some(s) => (s.size, false, Some(s)),
            None => measure(),
        };
        total += size;
        let candidate = i >= retention.keep && total > retention.budget;
        // Nothing is removed on the index's word: an entry is measured again,
        // so one written to since it settled is seen to be live.
        if candidate && known.is_some() {
            let size_before = size;
            (size, recent, settles) = measure();
            total = total - size_before + size;
        }
        if !candidate || total <= retention.budget || recent {
            if let Some(s) = settles {
                settled.insert(name, s);
            }
            continue;
        }
        // Gone either way, a concurrent prune having got there first.
        match evict(entry) {
            // A run still going is left whole, its files counted, and its
            // size kept if it had settled.
            Ok(false) => {
                if let Some(s) = settles {
                    settled.insert(name, s);
                }
            }
            // Part of it may be gone, so it is measured again next time.
            Err(e) if e.kind() != io::ErrorKind::NotFound => {}
            _ => total -= size,
        }
    }
    if settled != index {
        write_index(&index_path, &settled);
    }
    prune_records(root, retention, current);
}

/// Whether the entry at `dir` holds a build's files, or only its record (a
/// task that never built, or one evicted).
fn holds_files(dir: &Path) -> bool {
    dir.join(BAZEL_DIR).is_dir()
}

/// Remove an entry's files but its record, which is kept, its command line
/// redacted ([`TaskRecord::evict`]); one of a newer schema is kept as it is,
/// and an entry with no record goes whole. Whether it was evicted: a run
/// still running ([`task_record::still_running`]) is left whole (false),
/// since its process would write its record over the eviction's.
fn evict(entry: &Path) -> io::Result<bool> {
    let record_path = entry.join(task_record::FILE_NAME);
    if !record_path.is_file() {
        return fs::remove_dir_all(entry).map(|()| true);
    }
    // Under the entry's lock, which the run's own writes hold too.
    task_record::with_entry_lock(entry, || evict_locked(entry, &record_path))
}

fn evict_locked(entry: &Path, record_path: &Path) -> io::Result<bool> {
    let record = task_record::read(record_path);
    // Its process would write its record over the eviction's at its end.
    let written = fs::metadata(record_path).and_then(|m| m.modified()).ok();
    if record
        .as_ref()
        .is_some_and(|r| task_record::still_running(r, written))
    {
        return Ok(false);
    }
    for e in fs::read_dir(entry)?.filter_map(|e| e.ok()) {
        if e.file_name() == task_record::FILE_NAME {
            continue;
        }
        let path = e.path();
        let removed = if e.file_type().is_ok_and(|t| t.is_dir()) {
            fs::remove_dir_all(&path)
        } else {
            fs::remove_file(&path)
        };
        match removed {
            Err(err) if err.kind() != io::ErrorKind::NotFound => return Err(err),
            _ => {}
        }
    }
    // A record this CLI cannot read (a newer one's) is left as it is.
    let Some(mut record) = record else {
        return Ok(true);
    };
    record.evict(SystemTime::now());
    let bytes = serde_json::to_vec_pretty(&record).map_err(io::Error::other)?;
    task_record::write_private(record_path, &bytes).map(|()| true)
}

/// Remove the records of runs with no files past [`Retention::records`] of
/// them, the most recently started first, or started over
/// [`Retention::record_age`] ago; never `current`, nor one written to within
/// [`Retention::guard`], nor one still running
/// ([`task_record::still_running`]). They count toward neither the budget
/// nor [`Retention::keep`], which are for files.
fn prune_records(root: &Path, retention: Retention, current: &Path) {
    let Ok(read) = fs::read_dir(root) else {
        return;
    };
    let mut records: Vec<PathBuf> = read
        .filter_map(|e| e.ok())
        .filter(|e| e.file_type().is_ok_and(|t| t.is_dir()))
        .filter(|e| is_invocation_name(&e.file_name()))
        .map(|e| e.path())
        .filter(|p| p != current && !holds_files(p))
        .collect();
    records.sort();
    records.reverse();
    let oldest = SystemTime::now().checked_sub(retention.record_age);
    for (i, record) in records.iter().enumerate() {
        let name = record.file_name().unwrap().to_string_lossy().into_owned();
        let too_old = match (started(&name), oldest) {
            (Some(started), Some(oldest)) => started < oldest,
            _ => false,
        };
        if i < retention.records && !too_old {
            continue;
        }
        let (_, newest) = tree_size_and_newest(record);
        if newest.is_none_or(|t| !idle_for(t, retention.guard)) {
            continue;
        }
        // Nor a run still going, however long it has been quiet.
        let path = record.join(task_record::FILE_NAME);
        let written = fs::metadata(&path).and_then(|m| m.modified()).ok();
        if task_record::read(&path).is_some_and(|r| task_record::still_running(&r, written)) {
            continue;
        }
        let _ = fs::remove_dir_all(record);
    }
}

/// When the entry named `name` started, from its [`stamp`].
fn started(name: &str) -> Option<SystemTime> {
    let stamp = name.get(..STAMP_LEN)?;
    let at = chrono::NaiveDateTime::parse_from_str(stamp, "%Y%m%dT%H%M%S%3fZ").ok()?;
    Some(SystemTime::from(at.and_utc()))
}

/// Whether `time` is at least `long` ago. A time ahead of the clock is not.
fn idle_for(time: SystemTime, long: Duration) -> bool {
    SystemTime::now()
        .duration_since(time)
        .is_ok_and(|ago| ago >= long)
}

/// `time` in whole seconds since the epoch.
fn secs(time: SystemTime) -> u64 {
    time.duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}

/// The [`SIZES_INDEX`] at `path`: `<name> <size> <newest>` lines. A missing
/// or unreadable index, or a line that does not parse, is as good as none.
fn read_index(path: &Path) -> HashMap<String, Settled> {
    let Ok(text) = fs::read_to_string(path) else {
        return HashMap::new();
    };
    text.lines()
        .filter_map(|line| {
            let mut fields = line.split(' ');
            let name = fields.next()?;
            let size = fields.next()?.parse().ok()?;
            let newest = fields.next()?.parse().ok()?;
            (is_invocation_name(name.as_ref()) && fields.next().is_none())
                .then(|| (name.to_owned(), Settled { size, newest }))
        })
        .collect()
}

/// Replace the index at `path` whole, through a temp file renamed over it,
/// so a reader never sees half of one. Best effort, as the index is a cache.
fn write_index(path: &Path, settled: &HashMap<String, Settled>) {
    let mut names: Vec<&String> = settled.keys().collect();
    names.sort();
    let text: String = names
        .into_iter()
        .map(|name| {
            let s = settled[name];
            format!("{name} {} {}\n", s.size, s.newest)
        })
        .collect();
    let temp = path.with_extension(format!("{}.tmp", uuid::Uuid::new_v4()));
    if fs::write(&temp, text).is_ok() && fs::rename(&temp, path).is_err() {
        let _ = fs::remove_file(&temp);
    }
}

/// The length of [`stamp`]'s `YYYYMMDDTHHMMSSmmmZ`.
const STAMP_LEN: usize = 19;

/// `time` in UTC as `YYYYMMDDTHHMMSSmmmZ`: sortable as text, readable, and a
/// valid file name everywhere.
fn stamp(time: SystemTime) -> String {
    chrono::DateTime::<chrono::Utc>::from(time)
        .format("%Y%m%dT%H%M%S%3fZ")
        .to_string()
}

/// An invocation directory's name: a [`stamp`], `-`, and a task ID, which is a
/// UUID. Nothing else under the root is the history's to prune.
fn is_invocation_name(name: &std::ffi::OsStr) -> bool {
    let Some(name) = name.to_str() else {
        return false;
    };
    let b = name.as_bytes();
    b.len() > STAMP_LEN
        && b[..STAMP_LEN].iter().enumerate().all(|(i, c)| match i {
            8 => *c == b'T',
            18 => *c == b'Z',
            _ => c.is_ascii_digit(),
        })
        && b[STAMP_LEN] == b'-'
        && uuid::Uuid::parse_str(&name[STAMP_LEN + 1..]).is_ok()
}

/// The bytes under `dir` and the latest modification time in it, of a file or
/// a directory, `dir` included: a folder a build has just made counts before
/// its first file, as does an entry another task is still creating.
fn tree_size_and_newest(dir: &Path) -> (u64, Option<SystemTime>) {
    let mut size = 0;
    let mut newest: Option<SystemTime> = fs::metadata(dir).and_then(|m| m.modified()).ok();
    let mut stack = vec![dir.to_path_buf()];
    while let Some(dir) = stack.pop() {
        let Ok(read) = fs::read_dir(&dir) else {
            continue;
        };
        for entry in read.filter_map(|e| e.ok()) {
            let Ok(meta) = entry.metadata() else { continue };
            if let Ok(modified) = meta.modified() {
                newest = Some(newest.map_or(modified, |n| n.max(modified)));
            }
            if meta.is_dir() {
                stack.push(entry.path());
            } else {
                size += meta.len();
            }
        }
    }
    (size, newest)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The `n`th test task ID.
    fn id(n: u32) -> String {
        format!("00000000-0000-0000-0000-{n:012}")
    }

    /// The entry name of task `n`, started `n` seconds after the epoch: a
    /// larger `n` started later.
    fn name(n: u32) -> String {
        format!(
            "{}-{}",
            stamp(UNIX_EPOCH + Duration::from_secs(n as u64)),
            id(n)
        )
    }

    /// An invocation entry for task `n`, holding one file of `size` bytes, all
    /// of it last written `age` ago.
    fn entry(root: &Path, n: u32, size: u64, age: Duration) -> PathBuf {
        let dir = root.join(name(n));
        let log = bazel_dir(&dir, 1).join(bazel::execlog::FILE_NAME);
        fs::create_dir_all(log.parent().unwrap()).unwrap();
        let file = fs::File::create(&log).unwrap();
        file.set_len(size).unwrap();
        let when = SystemTime::now() - age;
        file.set_modified(when).unwrap();
        for d in [log.parent().unwrap(), &dir.join(BAZEL_DIR), &dir] {
            fs::File::open(d).unwrap().set_modified(when).unwrap();
        }
        dir
    }

    const OLD: Duration = Duration::from_secs(2 * 60 * 60);

    fn history(root: &Path, budget: u64, keep: usize) -> History {
        History::at(
            root.to_path_buf(),
            Retention {
                budget,
                keep,
                guard: Duration::from_secs(60 * 60),
                every: Duration::ZERO,
                settle: Duration::from_secs(60 * 60),
                records: 5000,
                record_age: Duration::from_secs(90 * 24 * 60 * 60),
            },
        )
    }

    /// Give the entry at `dir` a record with a secret on its command line,
    /// last written `age` ago.
    fn with_record(dir: &Path, age: Duration) {
        let record = TaskRecord {
            schema: task_record::SCHEMA,
            status: "ended".into(),
            argv: vec!["aspect".into(), "deploy".into(), "--token=s3cret".into()],
            secret_flags: vec!["--token".into()],
            ..Default::default()
        };
        let path = dir.join(task_record::FILE_NAME);
        fs::create_dir_all(dir).unwrap();
        fs::write(&path, serde_json::to_vec(&record).unwrap()).unwrap();
        let when = SystemTime::now() - age;
        fs::File::open(&path).unwrap().set_modified(when).unwrap();
        fs::File::open(dir).unwrap().set_modified(when).unwrap();
    }

    /// The next Bazel invocation's folder, once the prune it started is done.
    fn next(h: &History) -> PathBuf {
        let dir = h.next_bazel_dir().unwrap();
        h.wait_for_prune();
        dir
    }

    /// A prune over budget removes an entry's files but keeps its record,
    /// its command line redacted.
    #[test]
    fn an_evicted_entry_keeps_its_record_redacted() {
        let root = tempfile::tempdir().unwrap();
        let old = entry(root.path(), 1, 1000, OLD);
        with_record(&old, OLD);
        let h = history(root.path(), 1, 0);
        h.begin(&id(99));
        next(&h);
        assert!(!old.join(BAZEL_DIR).exists(), "its files are gone");
        let record = task_record::read(&old.join(task_record::FILE_NAME)).unwrap();
        assert!(record.evicted.is_some() && record.argv_redacted);
        assert_eq!(record.argv[2], "--token=<REDACTED>");
        let names: Vec<_> = fs::read_dir(&old)
            .unwrap()
            .map(|e| e.unwrap().file_name())
            .collect();
        assert_eq!(names, [std::ffi::OsString::from(task_record::FILE_NAME)]);
    }

    /// A run whose record says it is running, in a live process, keeps its
    /// files: that process would write its record over the eviction's.
    #[cfg(unix)]
    #[test]
    fn a_live_run_is_not_evicted() {
        let root = tempfile::tempdir().unwrap();
        let live = entry(root.path(), 1, 1000, OLD);
        let record = TaskRecord {
            schema: task_record::SCHEMA,
            status: "running".into(),
            // The parent stands in for another live process.
            pid: std::os::unix::process::parent_id(),
            ..Default::default()
        };
        fs::write(
            live.join(task_record::FILE_NAME),
            serde_json::to_vec(&record).unwrap(),
        )
        .unwrap();
        for p in [live.join(task_record::FILE_NAME), live.clone()] {
            fs::File::open(&p)
                .unwrap()
                .set_modified(SystemTime::now() - OLD)
                .unwrap();
        }
        let h = history(root.path(), 1, 0);
        h.begin(&id(99));
        next(&h);
        assert!(live.join(BAZEL_DIR).exists());
    }

    /// The records cap spares a run still going, however quiet.
    #[cfg(unix)]
    #[test]
    fn the_records_cap_spares_a_live_run() {
        let root = tempfile::tempdir().unwrap();
        let live = root.path().join(name(1));
        with_record(&live, OLD);
        let record = TaskRecord {
            schema: task_record::SCHEMA,
            status: "running".into(),
            pid: std::os::unix::process::parent_id(),
            ..Default::default()
        };
        let path = live.join(task_record::FILE_NAME);
        fs::write(&path, serde_json::to_vec(&record).unwrap()).unwrap();
        for p in [&path, &live] {
            fs::File::open(p)
                .unwrap()
                .set_modified(SystemTime::now() - OLD)
                .unwrap();
        }
        let mut h = history(root.path(), 1 << 30, 0);
        h.retention.records = 0;
        h.begin(&id(99));
        next(&h);
        assert!(live.exists());
    }

    /// A link is kept scrubbed of the task's secrets and URL credentials.
    #[test]
    fn a_link_is_kept_without_secrets() {
        let root = tempfile::tempdir().unwrap();
        let h = history(root.path(), 1 << 30, 10);
        h.set_command_line(
            vec!["aspect".into(), "--token=S3CRET".into()],
            vec!["--token".into()],
            vec![],
        );
        h.begin_task(&id(1), TaskStart::default());
        h.link("Deploy S3CRET", "https://user:pw@host/x?t=S3CRET");
        let record = h.record(&id(1)).unwrap().1;
        assert_eq!(
            (record.links[0].label.as_str(), record.links[0].url.as_str()),
            (
                "Deploy <REDACTED>",
                "https://<REDACTED>@host/x?t=<REDACTED>"
            )
        );
    }

    /// A run that builds nothing still caps the records at its end.
    #[test]
    fn a_run_without_a_build_caps_the_records() {
        let root = tempfile::tempdir().unwrap();
        for n in 1..=3 {
            with_record(&root.path().join(name(n)), OLD);
        }
        let mut h = history(root.path(), 1 << 30, 0);
        h.retention.records = 1;
        h.retention.record_age = Duration::from_secs(u32::MAX as u64 * 100);
        h.begin_task(&id(99), TaskStart::default());
        h.end_task(Some(0), String::new(), String::new(), false);
        let kept: Vec<bool> = (1..=3)
            .map(|n| root.path().join(name(n)).exists())
            .collect();
        assert_eq!(kept, [false, false, true]);
        assert!(h.root().join(RECORDS_PRUNED_MARKER).exists());
    }

    /// A record this CLI cannot read, a newer one's, is kept as it is.
    #[test]
    fn eviction_keeps_a_newer_record_as_it_is() {
        let root = tempfile::tempdir().unwrap();
        let old = entry(root.path(), 1, 1000, OLD);
        let newer = br#"{"schema": 99, "argv": ["aspect"]}"#;
        fs::write(old.join(task_record::FILE_NAME), newer).unwrap();
        fs::File::open(&old)
            .unwrap()
            .set_modified(SystemTime::now() - OLD)
            .unwrap();
        fs::File::open(old.join(task_record::FILE_NAME))
            .unwrap()
            .set_modified(SystemTime::now() - OLD)
            .unwrap();
        let h = history(root.path(), 1, 0);
        h.begin(&id(99));
        next(&h);
        assert!(!old.join(BAZEL_DIR).exists());
        assert_eq!(fs::read(old.join(task_record::FILE_NAME)).unwrap(), newer);
    }

    /// Records without files count toward neither the budget nor `keep`.
    #[test]
    fn records_alone_take_no_place_from_files() {
        let root = tempfile::tempdir().unwrap();
        let files = entry(root.path(), 1, 100, OLD);
        for n in 2..=4 {
            with_record(&root.path().join(name(n)), OLD);
        }
        // `keep` 1 protects the newest entry with files, not a newer record.
        // Started at the epoch: no record is too old for this test.
        let mut h = history(root.path(), 1, 1);
        h.retention.record_age = Duration::from_secs(u32::MAX as u64 * 100);
        h.begin(&id(99));
        next(&h);
        assert!(files.join(BAZEL_DIR).exists());
        for n in 2..=4 {
            assert!(
                root.path()
                    .join(name(n))
                    .join(task_record::FILE_NAME)
                    .exists()
            );
        }
    }

    /// Records past the count, or started too long ago, go; one written
    /// within the guard stays.
    #[test]
    fn records_are_capped_by_count_and_age() {
        let root = tempfile::tempdir().unwrap();
        // Started at the epoch onwards: past any age but the test's.
        for n in 1..=5 {
            with_record(&root.path().join(name(n)), OLD);
        }
        let live = root.path().join(name(6));
        with_record(&live, Duration::from_secs(60));
        let mut h = history(root.path(), 1 << 30, 0);
        h.retention.records = 2;
        h.retention.record_age = Duration::from_secs(u32::MAX as u64 * 100);
        h.begin(&id(99));
        next(&h);
        let kept: Vec<bool> = (1..=6)
            .map(|n| root.path().join(name(n)).exists())
            .collect();
        assert_eq!(
            kept,
            [false, false, false, false, true, true],
            "the newest two by start"
        );
        h.retention.record_age = Duration::from_secs(60 * 60);
        let other = History::at(root.path().to_path_buf(), h.retention);
        other.begin(&id(98));
        next(&other);
        assert!(!root.path().join(name(5)).exists(), "started too long ago");
        assert!(live.exists(), "written within the guard");
    }

    #[test]
    fn an_entrys_start_is_read_from_its_name() {
        assert_eq!(started(&name(7)), Some(UNIX_EPOCH + Duration::from_secs(7)));
        assert_eq!(started("logs"), None);
    }

    #[test]
    fn a_stamp_is_utc_with_milliseconds_and_sorts_as_text() {
        let at = |ms: u64| stamp(UNIX_EPOCH + Duration::from_millis(ms));
        assert_eq!(at(0), "19700101T000000000Z");
        // 2000-02-29, a leap day, 03:45:01.123.
        assert_eq!(at(951_795_901_123), "20000229T034501123Z");
        assert!(at(951_795_901_123) < at(951_795_901_124));
        assert_eq!(at(0).len(), STAMP_LEN);
    }

    #[test]
    fn bazel_invocations_get_numbered_folders_under_one_task_invocation() {
        let root = tempfile::tempdir().unwrap();
        let h = history(root.path(), 1 << 30, 10);
        h.begin(&id(1));
        let first = next(&h);
        let second = next(&h);
        assert!(first.ends_with("bazel/1") && second.ends_with("bazel/2"));
        assert_eq!(first.parent(), second.parent());
        let entry = first
            .parent()
            .unwrap()
            .parent()
            .unwrap()
            .file_name()
            .unwrap();
        assert!(is_invocation_name(entry), "{entry:?}");
        assert!(entry.to_str().unwrap().ends_with(&format!("-{}", id(1))));
    }

    #[test]
    fn nothing_is_created_until_a_bazel_invocation_asks() {
        let root = tempfile::tempdir().unwrap();
        let h = history(&root.path().join("history"), 1 << 30, 10);
        h.begin(&id(1));
        assert!(!h.root().exists());
    }

    #[test]
    fn the_oldest_started_go_first_once_over_budget() {
        let root = tempfile::tempdir().unwrap();
        let entries: Vec<_> = (1..=5).map(|n| entry(root.path(), n, 100, OLD)).collect();
        // Room for three of the five.
        let h = history(root.path(), 300, 0);
        h.begin(&id(99));
        next(&h);
        let kept: Vec<bool> = entries.iter().map(|e| e.exists()).collect();
        assert_eq!(kept, [false, false, true, true, true]);
    }

    #[test]
    fn the_most_recent_are_kept_whatever_their_size() {
        let root = tempfile::tempdir().unwrap();
        let entries: Vec<_> = (1..=4).map(|n| entry(root.path(), n, 1000, OLD)).collect();
        let h = history(root.path(), 1, 2);
        h.begin(&id(99));
        next(&h);
        let kept: Vec<bool> = entries.iter().map(|e| e.exists()).collect();
        assert_eq!(kept, [false, false, true, true]);
    }

    #[test]
    fn an_entry_written_to_within_the_guard_is_never_pruned() {
        let root = tempfile::tempdir().unwrap();
        let recent = entry(root.path(), 1, 1000, Duration::from_secs(60));
        let stale = entry(root.path(), 2, 1000, OLD);
        let h = history(root.path(), 1, 0);
        h.begin(&id(99));
        next(&h);
        assert!(recent.exists(), "still being written or uploaded, maybe");
        assert!(!stale.exists());
    }

    #[test]
    fn only_the_first_bazel_invocation_prunes() {
        let root = tempfile::tempdir().unwrap();
        let h = history(root.path(), 1, 0);
        h.begin(&id(99));
        next(&h);
        let late = entry(root.path(), 1, 1000, OLD);
        next(&h);
        assert!(late.exists());
    }

    #[test]
    fn a_folder_a_build_has_just_made_is_recent_before_its_first_file() {
        let root = tempfile::tempdir().unwrap();
        // A later-started, recent entry keeps the total over budget when the
        // empty folder is reached, so only the guard can keep it.
        let fresh = bazel_dir(&root.path().join(name(1)), 1);
        fs::create_dir_all(&fresh).unwrap();
        entry(root.path(), 2, 1000, Duration::from_secs(60));
        let h = history(root.path(), 1, 0);
        h.begin(&id(99));
        next(&h);
        assert!(fresh.exists(), "the build may not have opened its log yet");
    }

    #[test]
    fn nothing_but_invocation_folders_is_pruned() {
        let root = tempfile::tempdir().unwrap();
        let other = root.path().join("not-ours");
        fs::create_dir_all(&other).unwrap();
        let file = other.join("big");
        fs::File::create(&file).unwrap().set_len(1000).unwrap();
        fs::File::options()
            .write(true)
            .open(&file)
            .unwrap()
            .set_modified(SystemTime::now() - OLD)
            .unwrap();
        let h = history(root.path(), 1, 0);
        h.begin(&id(99));
        next(&h);
        assert!(other.exists());
    }

    #[test]
    fn an_invocation_name_is_a_stamp_and_a_task_id() {
        assert!(is_invocation_name(name(1).as_ref()));
        assert!(!is_invocation_name(id(1).as_ref()), "a bare task id");
        assert!(
            !is_invocation_name("20000229T034501123Z-abc".as_ref()),
            "not a uuid"
        );
        assert!(!is_invocation_name("logs".as_ref()));
        assert!(!is_invocation_name("é".as_ref()));
    }

    #[cfg(unix)]
    #[test]
    fn the_history_is_readable_by_its_user_alone() {
        use std::os::unix::fs::PermissionsExt;
        let root = tempfile::tempdir().unwrap();
        let h = history(&root.path().join("history"), 1 << 30, 10);
        h.begin(&id(1));
        let mut dir = next(&h);
        loop {
            let mode = fs::metadata(&dir).unwrap().permissions().mode() & 0o777;
            assert_eq!(mode, 0o700, "{}", dir.display());
            if dir == root.path().join("history") {
                break;
            }
            dir.pop();
        }
    }

    #[test]
    fn an_earlier_run_of_a_task_id_is_found() {
        let root = tempfile::tempdir().unwrap();
        let h = history(root.path(), 1 << 30, 10);
        assert_eq!(h.earlier_run(&id(1)), None, "no history yet");
        h.begin(&id(1));
        let dir = next(&h);
        let entry = dir.parent().unwrap().parent().unwrap().to_path_buf();
        assert_eq!(h.earlier_run(&id(1)), Some(entry));
        assert_eq!(h.earlier_run(&id(2)), None);
        assert!(
            h.earlier_run(&id(1).to_uppercase()).is_some(),
            "a UUID in another case is the same ID"
        );
    }

    #[test]
    fn another_invocations_prune_spares_one_written_within_the_guard() {
        let root = tempfile::tempdir().unwrap();
        let h = history(root.path(), 1, 0);
        h.begin(&id(1));
        let dir = next(&h);
        fs::write(dir.join(bazel::execlog::FILE_NAME), vec![0u8; 1000]).unwrap();
        let other = History::at(root.path().to_path_buf(), h.retention);
        other.begin(&id(2));
        next(&other);
        assert!(dir.exists(), "written within the guard");
    }

    /// A prune by any task within `every` makes the next one wait.
    #[test]
    fn a_recent_prune_makes_the_next_skip() {
        let root = tempfile::tempdir().unwrap();
        let mut h = history(root.path(), 1, 0);
        h.retention.every = Duration::from_secs(60 * 60);
        h.begin(&id(99));
        next(&h);
        let old = entry(root.path(), 1, 1000, OLD);
        let other = History::at(root.path().to_path_buf(), h.retention);
        other.begin(&id(98));
        next(&other);
        assert!(old.exists(), "pruned too recently to prune again");
    }

    /// A settled entry's size comes from the index, not its files: a newer
    /// one the index says is large pushes an older one out, though the files
    /// of both are small.
    #[test]
    fn a_settled_entry_is_measured_once() {
        let root = tempfile::tempdir().unwrap();
        let older = entry(root.path(), 1, 10, OLD);
        entry(root.path(), 2, 10, OLD);
        // Kept whatever their size: this first task's own entry, and entry 2.
        let h = history(root.path(), 100, 2);
        h.begin(&id(99));
        next(&h);
        assert!(older.exists(), "under budget");
        let index = fs::read_to_string(root.path().join(SIZES_INDEX)).unwrap();
        let inflated = index.replace(&format!("{} 10 ", name(2)), &format!("{} 1000 ", name(2)));
        assert_ne!(index, inflated, "{index}");
        fs::write(root.path().join(SIZES_INDEX), inflated).unwrap();
        let other = History::at(root.path().to_path_buf(), h.retention);
        other.begin(&id(98));
        next(&other);
        assert!(!older.exists(), "the index's size is the one counted");
    }

    /// An entry is never removed on the index's word: one written to in place
    /// since it settled is measured again and found live.
    #[test]
    fn a_settled_entry_written_to_again_is_kept() {
        let root = tempfile::tempdir().unwrap();
        let settled = entry(root.path(), 1, 10, OLD);
        let h = history(root.path(), 100, 0);
        h.begin(&id(99));
        next(&h);
        let log = bazel_dir(&settled, 1).join(bazel::execlog::FILE_NAME);
        let file = fs::File::options().write(true).open(&log).unwrap();
        file.set_len(1000).unwrap();
        file.set_modified(SystemTime::now()).unwrap();
        // Kept whatever its size, with the first task's own entry, so the
        // settled one is what a prune would remove.
        entry(root.path(), 2, 1000, OLD);
        let mut retention = h.retention;
        retention.keep = 2;
        let other = History::at(root.path().to_path_buf(), retention);
        other.begin(&id(98));
        next(&other);
        assert!(settled.exists(), "written within the guard");
    }

    /// An entry idle past the guard but not yet `settle` is measured each
    /// time, not indexed.
    #[test]
    fn an_entry_settles_only_once_idle_long_enough() {
        let root = tempfile::tempdir().unwrap();
        entry(root.path(), 1, 10, OLD);
        let mut h = history(root.path(), 100, 0);
        h.retention.settle = Duration::from_secs(24 * 60 * 60);
        h.begin(&id(99));
        next(&h);
        let index = fs::read_to_string(root.path().join(SIZES_INDEX)).unwrap_or_default();
        assert!(!index.contains(&name(1)), "{index}");
    }

    /// A marker ahead of the clock, as after a clock set back, has expired.
    #[test]
    fn a_marker_from_the_future_does_not_stop_pruning() {
        let root = tempfile::tempdir().unwrap();
        let old = entry(root.path(), 1, 1000, OLD);
        let marker = root.path().join(PRUNED_MARKER);
        fs::write(&marker, b"").unwrap();
        fs::File::open(&marker)
            .unwrap()
            .set_modified(SystemTime::now() + Duration::from_secs(365 * 24 * 60 * 60))
            .unwrap();
        let mut h = history(root.path(), 1, 0);
        h.retention.every = Duration::from_secs(60 * 60);
        h.begin(&id(99));
        next(&h);
        assert!(!old.exists());
    }

    /// A further Bazel invocation of a settled entry makes a new folder under
    /// its `bazel`, so it is measured again.
    #[test]
    fn a_settled_entry_that_grows_is_measured_again() {
        let root = tempfile::tempdir().unwrap();
        let old = entry(root.path(), 1, 10, OLD);
        let h = history(root.path(), 100, 0);
        h.begin(&id(99));
        next(&h);
        let grown = bazel_dir(&old, 2);
        fs::create_dir_all(&grown).unwrap();
        fs::File::create(grown.join(bazel::execlog::FILE_NAME))
            .unwrap()
            .set_len(1000)
            .unwrap();
        let other = History::at(root.path().to_path_buf(), h.retention);
        other.begin(&id(98));
        next(&other);
        assert!(old.exists(), "written within the guard");
        let index = fs::read_to_string(root.path().join(SIZES_INDEX)).unwrap();
        assert!(
            !index.contains(&name(1)),
            "live again, so not settled: {index}"
        );
    }

    /// An index that does not parse is as good as none.
    #[test]
    fn a_damaged_index_is_rebuilt() {
        let root = tempfile::tempdir().unwrap();
        fs::create_dir_all(root.path()).unwrap();
        fs::write(root.path().join(SIZES_INDEX), "not an index\n\0\0").unwrap();
        let old = entry(root.path(), 1, 1000, OLD);
        entry(root.path(), 2, 10, OLD);
        let h = history(root.path(), 100, 0);
        h.begin(&id(99));
        next(&h);
        assert!(!old.exists());
        let index = fs::read_to_string(root.path().join(SIZES_INDEX)).unwrap();
        assert_eq!(
            index,
            format!(
                "{} 10 {}\n",
                name(2),
                index.split(' ').nth(2).unwrap().trim()
            )
        );
    }

    /// An entry the budget would remove is measured first, and one smaller
    /// than the index says is kept when its real size fits.
    #[test]
    fn a_removal_is_decided_on_a_fresh_measure() {
        let root = tempfile::tempdir().unwrap();
        let settled = entry(root.path(), 1, 10, OLD);
        let h = history(root.path(), 100, 0);
        h.begin(&id(99));
        next(&h);
        let index = fs::read_to_string(root.path().join(SIZES_INDEX)).unwrap();
        let inflated = index.replace(&format!("{} 10 ", name(1)), &format!("{} 1000 ", name(1)));
        assert_ne!(index, inflated, "{index}");
        fs::write(root.path().join(SIZES_INDEX), inflated).unwrap();
        let other = History::at(root.path().to_path_buf(), h.retention);
        other.begin(&id(98));
        next(&other);
        assert!(settled.exists(), "10 bytes fit the budget");
    }

    /// An index's temp file left by a cut-off write goes once past the guard.
    #[test]
    fn a_stale_index_temp_file_is_removed() {
        let root = tempfile::tempdir().unwrap();
        let (stale, fresh) = (
            root.path().join(format!("{SIZES_INDEX}.a.tmp")),
            root.path().join(format!("{SIZES_INDEX}.b.tmp")),
        );
        for path in [&stale, &fresh] {
            fs::write(path, b"").unwrap();
        }
        fs::File::open(&stale)
            .unwrap()
            .set_modified(SystemTime::now() - OLD)
            .unwrap();
        let h = history(root.path(), 100, 0);
        h.begin(&id(99));
        next(&h);
        assert!(!stale.exists());
        assert!(fresh.exists(), "may be a write still in flight");
    }

    /// An entry another task has made but not yet filled is recent by its own
    /// folder's time.
    #[test]
    fn an_entry_folder_being_made_is_recent() {
        let root = tempfile::tempdir().unwrap();
        let being_made = root.path().join(name(1));
        fs::create_dir_all(&being_made).unwrap();
        // Kept whatever its size, so the one being made is what a prune
        // would remove.
        entry(root.path(), 2, 1000, OLD);
        let h = history(root.path(), 1, 1);
        h.begin(&id(99));
        next(&h);
        assert!(being_made.exists(), "its bazel folder is not made yet");
    }
}
