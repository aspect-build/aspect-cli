//! Each task invocation's files, kept for later: one directory per invocation
//! under the history root, named `<start>-<task-id>` where `<start>` is the UTC
//! start time as `YYYYMMDDTHHMMSSmmmZ`, so the names alone list the history in
//! order. A subfolder per Bazel invocation it makes (`bazel/<n>`). A task ID
//! names one run, so an explicit one already here is refused (see
//! [`History::earlier_run`]). An entry is made only for a Bazel invocation that
//! will keep a file in it, at its spawn, so a run that keeps nothing leaves no
//! trace; one that fails before writing its files leaves an empty folder.
//!
//! A Bazel invocation's folder holds its compact execution log, workspace log,
//! trace profile and build event stream, each when the build has one (see
//! `ctx.bazel.build`). The folders are readable by their user alone, since an
//! execution log holds action command lines and environment variables
//! unredacted; the build event stream is kept as the CLI redacted it.
//!
//! Bazel reads some of these files back after the build — it uploads the
//! compact execution log as a build tool log, possibly after the client has
//! exited — so nothing here is moved or deleted by the invocation that wrote
//! it. A [`prune`] removes old invocations instead, started in the background
//! by an invocation's first Bazel build unless another task pruned within
//! [`Retention::every`]: the oldest started first while the history is over
//! [`Retention::budget`], never below [`Retention::keep`] invocations, and
//! never one written to within [`Retention::guard`], which may be a build or an
//! upload still running in another workspace. Liveness is read from those times
//! alone, so a run elsewhere that has gone longer than the guard without
//! writing, and is not among the newest kept, can lose its entry. Two dotfiles
//! at the root serve it: `.pruned`, whose time is the last prune's, and
//! `.sizes`, the sizes of invocations that have settled.

use std::collections::HashMap;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::sync::atomic::{AtomicBool, Ordering};
use std::thread::JoinHandle;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

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
    /// Invocations always kept, the most recently started, whatever their size.
    pub keep: usize,
    /// An invocation written to this recently is never pruned.
    pub guard: Duration,
    /// A prune this recent, by any task, makes another wait.
    pub every: Duration,
}

impl Default for Retention {
    fn default() -> Self {
        Self {
            budget: 5 << 30,
            keep: 10,
            guard: Duration::from_secs(60 * 60),
            every: Duration::from_secs(10 * 60),
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

    /// Start the task invocation `task_id`, now. Nothing is created on disk
    /// until its first Bazel invocation asks for a directory.
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
        create_private_dir(&dir)?;
        if invocation.bazel_invocations == 1 && claim_prune(&self.root, self.retention.every) {
            let (root, retention, current) =
                (self.root.clone(), self.retention, invocation.dir.clone());
            let handle = std::thread::Builder::new()
                .name("history-prune".into())
                .spawn(move || prune(&root, retention, &current))
                .ok();
            *self.pruning.lock().unwrap() = handle;
        }
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

/// The sizes of settled invocations; see [`prune`].
const SIZES_INDEX: &str = ".sizes";

/// Whether this task prunes `root` now: when no task has within `every`. The
/// marker is touched first, so tasks started together prune once.
fn claim_prune(root: &Path, every: Duration) -> bool {
    let marker = root.join(PRUNED_MARKER);
    let recent = fs::metadata(&marker)
        .and_then(|m| m.modified())
        .is_ok_and(|t| SystemTime::now().duration_since(t).unwrap_or_default() < every);
    !recent && fs::write(&marker, b"").is_ok()
}

/// A settled invocation's size, and the newest modification time under it
/// when it was measured.
#[derive(Debug, Clone, Copy, PartialEq)]
struct Settled {
    size: u64,
    newest: u64,
}

/// Remove old invocations under `root` as `retention` says, leaving `current`
/// alone. Best effort: an entry that cannot be read or removed is skipped.
///
/// It runs beside the build and is not waited for: the CLI may exit midway,
/// which leaves only a valid history, part of an entry it was removing at
/// worst, which the next prune finishes.
///
/// An invocation nothing has written to within [`Retention::guard`] is
/// settled: its size is kept in the [`SIZES_INDEX`] and not measured again,
/// so a prune walks only new and live entries and checks each settled one's
/// `bazel` folder, which a further Bazel invocation of it would update. The
/// index is a cache, rebuilt from the entries whenever it is missing or
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
        .filter(|p| p != current)
        .collect();
    // Names start with the start time, so this is the most recent first.
    entries.sort();
    entries.reverse();
    let index_path = root.join(SIZES_INDEX);
    let index = read_index(&index_path);
    let mut settled = HashMap::new();
    let now = SystemTime::now();
    let within_guard = |t: SystemTime| now.duration_since(t).unwrap_or_default() < retention.guard;
    let mut total = 0u64;
    for (i, entry) in entries.iter().enumerate() {
        let name = entry.file_name().unwrap().to_string_lossy().into_owned();
        let known = index.get(&name).copied().filter(|s| {
            fs::metadata(entry.join(BAZEL_DIR))
                .and_then(|m| m.modified())
                .is_ok_and(|t| secs(t) <= s.newest)
        });
        let (size, recent) = match known {
            Some(s) => (s.size, false),
            None => {
                let (size, newest) = tree_size_and_newest(entry);
                let recent = newest.is_some_and(within_guard);
                if let (Some(newest), false) = (newest, recent) {
                    settled.insert(
                        name.clone(),
                        Settled {
                            size,
                            newest: secs(newest),
                        },
                    );
                }
                (size, recent)
            }
        };
        if let Some(s) = known {
            settled.insert(name.clone(), s);
        }
        total += size;
        if i < retention.keep || total <= retention.budget || recent {
            continue;
        }
        // Gone either way, a concurrent prune having got there first.
        match fs::remove_dir_all(entry) {
            Err(e) if e.kind() != io::ErrorKind::NotFound => {}
            _ => {
                total -= size;
                settled.remove(&name);
            }
        }
    }
    if settled != index {
        write_index(&index_path, &settled);
    }
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
    let since = time.duration_since(UNIX_EPOCH).unwrap_or_default();
    let (secs, ms) = (since.as_secs(), since.subsec_millis());
    let (days, of_day) = (secs / 86_400, secs % 86_400);
    let (year, month, day) = civil_from_days(days as i64);
    format!(
        "{year:04}{month:02}{day:02}T{:02}{:02}{:02}{ms:03}Z",
        of_day / 3600,
        of_day / 60 % 60,
        of_day % 60,
    )
}

/// The proleptic Gregorian date `days` after 1970-01-01 (Howard Hinnant's
/// `civil_from_days`).
fn civil_from_days(days: i64) -> (i64, u32, u32) {
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = (doy - (153 * mp + 2) / 5 + 1) as u32;
    let month = if mp < 10 { mp + 3 } else { mp - 9 } as u32;
    let year = yoe + era * 400 + i64::from(month <= 2);
    (year, month, day)
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
/// a directory: a folder a build has just made counts before its first file.
fn tree_size_and_newest(dir: &Path) -> (u64, Option<SystemTime>) {
    let mut size = 0;
    let mut newest: Option<SystemTime> = None;
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
            },
        )
    }

    /// The next Bazel invocation's folder, once the prune it started is done.
    fn next(h: &History) -> PathBuf {
        let dir = h.next_bazel_dir().unwrap();
        h.wait_for_prune();
        dir
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
    fn a_recent_prune_makes_the_next_wait() {
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

    /// A settled entry's size comes from the index, not its files: one the
    /// index says is large is pruned though its files are small.
    #[test]
    fn a_settled_entry_is_measured_once() {
        let root = tempfile::tempdir().unwrap();
        let old = entry(root.path(), 1, 10, OLD);
        let h = history(root.path(), 100, 0);
        h.begin(&id(99));
        next(&h);
        assert!(old.exists(), "under budget");
        let index = fs::read_to_string(root.path().join(SIZES_INDEX)).unwrap();
        assert!(index.starts_with(&format!("{} 10 ", name(1))), "{index}");
        fs::write(
            root.path().join(SIZES_INDEX),
            index.replace(" 10 ", " 1000 "),
        )
        .unwrap();
        let other = History::at(root.path().to_path_buf(), h.retention);
        other.begin(&id(98));
        next(&other);
        assert!(!old.exists(), "the index's size is the one counted");
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
}
