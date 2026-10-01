//! [`crate::Backend::Notify`]: in-process watcher via the `notify` crate.

use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use notify::event::{ModifyKind, RenameMode};
use notify::{
    Config as NotifyConfig, Event, EventKind, RecommendedWatcher, RecursiveMode, Watcher as _,
};

use crate::debounce::Pending;
use crate::{ChangeBatch, ChangeKind, Subscription, WatchConfig, WatchError, ignored, non_empty};

pub(crate) fn subscribe(config: WatchConfig) -> Result<Box<dyn Subscription>, WatchError> {
    let pending = Arc::new(Mutex::new(Pending::default()));
    let error = Arc::new(Mutex::new(None));
    let new_dirs = Arc::new(Mutex::new(Vec::new()));
    let watcher = create_watcher(&config, &pending, &error, &new_dirs)?;
    Ok(Box::new(NotifySubscription {
        watcher: Some(watcher),
        pending,
        error,
        new_dirs,
        config,
    }))
}

fn create_watcher(
    config: &WatchConfig,
    pending: &Arc<Mutex<Pending>>,
    error: &Arc<Mutex<Option<String>>>,
    new_dirs: &Arc<Mutex<Vec<PathBuf>>>,
) -> Result<RecommendedWatcher, WatchError> {
    let ignore_prefixes = config.ignore_prefixes.clone();
    let root = config.root.clone();
    let canonical_root = root
        .canonicalize()
        .map_err(|e| WatchError::Backend(format!("cannot canonicalize {root:?}: {e}")))?;
    let sink = Arc::clone(pending);
    let error_sink = Arc::clone(error);
    let new_dirs_sink = Arc::clone(new_dirs);
    let mut watcher = RecommendedWatcher::new(
        move |res: Result<Event, notify::Error>| match res {
            Ok(mut event) => {
                // Native backends may canonicalize watched paths (notably
                // FSEvents turns /tmp into /private/tmp). Re-root them at the
                // caller-spelled root so ignore prefixes and consumers see a
                // consistent path vocabulary across backends.
                for path in &mut event.paths {
                    if let Ok(relative) = path.strip_prefix(&canonical_root) {
                        *path = root.join(relative);
                    }
                }
                // A directory appearing directly under the root — created or
                // renamed in — needs its own recursive watch (the root
                // itself is watched non-recursively — see watch_root);
                // registration happens on the next try_recv, which owns the
                // watcher.
                if matches!(
                    event.kind,
                    EventKind::Create(_)
                        | EventKind::Modify(ModifyKind::Name(
                            RenameMode::To | RenameMode::Both | RenameMode::Any
                        ))
                ) {
                    for path in &event.paths {
                        if path.parent() == Some(root.as_path()) && !ignored(path, &ignore_prefixes)
                        {
                            new_dirs_sink.lock().unwrap().push(path.clone());
                        }
                    }
                }
                ingest(&mut sink.lock().unwrap(), &event, &ignore_prefixes)
            }
            // Backend errors (queue overflow, watch-limit exhaustion) mean
            // events may have been missed and the watch may stay degraded.
            // Surface the error through try_recv rather than silence.
            Err(e) => {
                error_sink.lock().unwrap().get_or_insert(e.to_string());
            }
        },
        NotifyConfig::default().with_follow_symlinks(false),
    )
    .map_err(|e| WatchError::Backend(e.to_string()))?;
    watch_root(&mut watcher, &config.root, &config.ignore_prefixes)?;
    Ok(watcher)
}

/// Watch `root` without recursing through ignored or symlinked entries.
///
/// A blanket recursive watch would traverse the bazel convenience symlinks
/// into the execroot on backends that follow symlinks while walking (inotify)
/// — `ignore_prefixes` filter events, not watch registrations, so that would
/// exhaust watch descriptors on large outputs. Instead: the root itself is
/// watched non-recursively (top-level files and new entries), and each
/// non-ignored, non-symlink top-level directory gets its own recursive watch.
fn watch_root(
    watcher: &mut RecommendedWatcher,
    root: &Path,
    ignore_prefixes: &[PathBuf],
) -> Result<(), WatchError> {
    watcher
        .watch(root, RecursiveMode::NonRecursive)
        .map_err(|e| WatchError::Backend(e.to_string()))?;
    let entries = std::fs::read_dir(root).map_err(|e| WatchError::Backend(e.to_string()))?;
    for entry in entries {
        let Ok(entry) = entry else { continue };
        let path = entry.path();
        if ignored(&path, ignore_prefixes) {
            continue;
        }
        let Ok(file_type) = entry.file_type() else {
            continue;
        };
        if file_type.is_dir() {
            watcher
                .watch(&path, RecursiveMode::Recursive)
                .map_err(|e| WatchError::Backend(e.to_string()))?;
        }
    }
    Ok(())
}

fn ingest(pending: &mut Pending, event: &Event, ignore_prefixes: &[PathBuf]) {
    if event.need_rescan() {
        pending.record_rescan();
        return;
    }
    let kinds: &[ChangeKind] = match event.kind {
        EventKind::Access(_) => return,
        EventKind::Create(_) => &[ChangeKind::Created],
        EventKind::Remove(_) => &[ChangeKind::Removed],
        EventKind::Modify(ModifyKind::Name(RenameMode::From)) => &[ChangeKind::Removed],
        EventKind::Modify(ModifyKind::Name(RenameMode::To)) => &[ChangeKind::Created],
        // Rename with both endpoints in one event: paths are [from, to].
        EventKind::Modify(ModifyKind::Name(RenameMode::Both)) => {
            &[ChangeKind::Removed, ChangeKind::Created]
        }
        // An ambiguous rename with both endpoints: same shape as Both. With a
        // single path the destination may be unreported (kqueue); Modified
        // plus the on-disk reconciliation below recovers the removal side.
        EventKind::Modify(ModifyKind::Name(RenameMode::Any)) if event.paths.len() == 2 => {
            &[ChangeKind::Removed, ChangeKind::Created]
        }
        EventKind::Modify(_) | EventKind::Any | EventKind::Other => &[ChangeKind::Modified],
    };
    for (i, path) in event.paths.iter().enumerate() {
        if ignored(path, ignore_prefixes) {
            continue;
        }
        // When kinds has one entry it applies to every path; otherwise zip.
        let kind = if kinds.len() == 1 {
            kinds[0]
        } else {
            kinds[i.min(kinds.len() - 1)]
        };
        pending.record(path.clone(), kind);
    }
}

/// FSEvents coalesces flags per path (a created-then-removed file can surface
/// as `Created`), so event kinds alone are untrustworthy. Settle each change
/// against what is actually on disk. Only a definitive NotFound downgrades to
/// Removed — a permission or transient I/O error keeps the reported kind.
fn reconcile(mut batch: ChangeBatch) -> ChangeBatch {
    for change in &mut batch.changes {
        change.kind = match std::fs::symlink_metadata(&change.path) {
            Ok(_) if change.kind == ChangeKind::Removed => ChangeKind::Modified,
            Ok(_) => change.kind,
            Err(e)
                if e.kind() == std::io::ErrorKind::NotFound
                    || e.kind() == std::io::ErrorKind::NotADirectory =>
            {
                ChangeKind::Removed
            }
            Err(_) => change.kind,
        };
    }
    batch
}

struct NotifySubscription {
    watcher: Option<RecommendedWatcher>,
    pending: Arc<Mutex<Pending>>,
    error: Arc<Mutex<Option<String>>>,
    new_dirs: Arc<Mutex<Vec<PathBuf>>>,
    config: WatchConfig,
}

impl Subscription for NotifySubscription {
    fn try_recv(&mut self) -> Result<Option<ChangeBatch>, WatchError> {
        if let Some(err) = self.error.lock().unwrap().take() {
            return Err(WatchError::Backend(err));
        }
        // Register recursive watches for directories that appeared under a
        // root since the last poll. Contents written before the watch lands
        // are covered by the directory's own Created event, which consumers
        // treat as a change to act on. Drained before registering: holding
        // the mutex across watch() could deadlock against the event-callback
        // thread taking the same lock.
        let pending_dirs: Vec<PathBuf> = self.new_dirs.lock().unwrap().drain(..).collect();
        for dir in pending_dirs {
            // symlink_metadata: a symlink to a directory must not get its
            // target recursively watched.
            let is_dir = std::fs::symlink_metadata(&dir)
                .map(|m| m.file_type().is_dir())
                .unwrap_or(false);
            if !is_dir {
                continue;
            }
            if let Err(e) = self
                .watcher
                .as_mut()
                .expect("watcher is present outside drain")
                .watch(&dir, RecursiveMode::Recursive)
            {
                // A vanished path is a benign race; anything else leaves the
                // subscription silently incomplete — surface it.
                if dir.exists() {
                    self.error.lock().unwrap().get_or_insert(e.to_string());
                }
            }
        }
        let batch = self
            .pending
            .lock()
            .unwrap()
            .try_take(self.config.debounce, self.config.max_delay);
        Ok(batch.map(reconcile).and_then(non_empty))
    }

    fn drain(&mut self) {
        // Dropping the old watcher stops its callback thread, providing a
        // barrier after every native event already queued for delivery. Clear
        // those events, then synchronously register a fresh watcher before
        // returning so subsequent writes are observed.
        drop(self.watcher.take());
        self.pending.lock().unwrap().clear();
        self.new_dirs.lock().unwrap().clear();
        self.error.lock().unwrap().take();
        match create_watcher(&self.config, &self.pending, &self.error, &self.new_dirs) {
            Ok(watcher) => {
                self.watcher = Some(watcher);
                // FSEvents may replay a just-past event into a newly created
                // stream. Give that initial delivery a chance to land before
                // the final clear that establishes the drain boundary.
                #[cfg(target_os = "macos")]
                std::thread::sleep(std::time::Duration::from_millis(100));
                self.pending.lock().unwrap().clear();
                self.new_dirs.lock().unwrap().clear();
            }
            Err(e) => {
                let WatchError::Backend(message) = e;
                self.error.lock().unwrap().replace(message);
            }
        }
    }
}
