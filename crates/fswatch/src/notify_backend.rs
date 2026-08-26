//! [`crate::Backend::Notify`]: in-process watcher via the `notify` crate.

use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use notify::event::{ModifyKind, RenameMode};
use notify::{Event, EventKind, RecommendedWatcher, RecursiveMode, Watcher as _};

use crate::debounce::Pending;
use crate::{
    ChangeBatch, ChangeKind, Subscription, WatchConfig, WatchError, drop_root_symlinks, ignored,
    non_empty,
};

pub(crate) fn subscribe(config: WatchConfig) -> Result<Box<dyn Subscription>, WatchError> {
    let pending = Arc::new(Mutex::new(Pending::default()));
    let error = Arc::new(Mutex::new(None));
    let new_dirs = Arc::new(Mutex::new(Vec::new()));
    let ignore_prefixes = config.ignore_prefixes.clone();
    let roots = config.roots.clone();
    let sink = Arc::clone(&pending);
    let error_sink = Arc::clone(&error);
    let new_dirs_sink = Arc::clone(&new_dirs);
    let mut watcher = notify::recommended_watcher(move |res: Result<Event, notify::Error>| {
        match res {
            Ok(event) => {
                // A directory appearing directly under a root — created or
                // renamed in — needs its own recursive watch (roots
                // themselves are watched non-recursively — see watch_root);
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
                        if path.parent().is_some_and(|p| roots.iter().any(|r| r == p))
                            && !ignored(path, &ignore_prefixes)
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
        }
    })
    .map_err(|e| WatchError::Backend(e.to_string()))?;
    for root in &config.roots {
        watch_root(&mut watcher, root, &config.ignore_prefixes)?;
    }
    Ok(Box::new(NotifySubscription {
        watcher,
        pending,
        error,
        new_dirs,
        config,
    }))
}

/// Watch `root` without recursing through ignored or symlinked entries.
///
/// A blanket recursive watch would traverse the bazel convenience symlinks
/// into the execroot on backends that follow symlinks while walking (inotify)
/// — `ignore_prefixes` filter events, not watch registrations, so that would
/// exhaust watch descriptors on large outputs. Instead: the root itself is
/// watched non-recursively (top-level files and new entries), and each
/// non-ignored, non-symlink top-level directory gets its own recursive watch.
/// Symlinks nested deeper than the top level are still followed; the
/// pathological case (bazel-*) lives at the root.
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
    watcher: RecommendedWatcher,
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
            if let Err(e) = self.watcher.watch(&dir, RecursiveMode::Recursive) {
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
        Ok(batch
            .map(reconcile)
            .map(|b| drop_root_symlinks(b, &self.config))
            .and_then(non_empty))
    }

    fn drain(&mut self) {
        self.pending.lock().unwrap().clear();
    }
}
