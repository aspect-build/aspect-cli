//! Backend-agnostic coalescing and debouncing of raw watch events.

use std::collections::HashMap;
use std::path::PathBuf;
use std::time::{Duration, Instant};

use crate::{Change, ChangeBatch, ChangeKind};

/// Pending changes shared between a backend's event thread and the consumer.
#[derive(Debug, Default)]
pub(crate) struct Pending {
    changes: HashMap<PathBuf, ChangeKind>,
    rescan: bool,
    first_event: Option<Instant>,
    last_event: Option<Instant>,
}

impl Pending {
    pub(crate) fn record(&mut self, path: PathBuf, kind: ChangeKind) {
        let now = Instant::now();
        self.first_event.get_or_insert(now);
        self.last_event = Some(now);
        match self.changes.remove(&path) {
            Some(prev) => {
                if let Some(merged) = merge(prev, kind) {
                    self.changes.insert(path, merged);
                }
            }
            None => {
                self.changes.insert(path, kind);
            }
        }
    }

    pub(crate) fn record_rescan(&mut self) {
        let now = Instant::now();
        self.first_event.get_or_insert(now);
        self.last_event = Some(now);
        self.rescan = true;
    }

    /// Release a batch if the debounce window has settled or `max_delay` is up.
    pub(crate) fn try_take(
        &mut self,
        debounce: Duration,
        max_delay: Duration,
    ) -> Option<ChangeBatch> {
        if self.changes.is_empty() && !self.rescan {
            // Nothing observable survived coalescing; forget the window so a
            // stale first_event can't trip max_delay for the next batch.
            self.first_event = None;
            self.last_event = None;
            return None;
        }
        let now = Instant::now();
        let settled = self
            .last_event
            .is_none_or(|t| now.duration_since(t) >= debounce);
        let overdue = self
            .first_event
            .is_some_and(|t| now.duration_since(t) >= max_delay);
        if !settled && !overdue {
            return None;
        }
        Some(self.take())
    }

    pub(crate) fn clear(&mut self) {
        self.take();
    }

    fn take(&mut self) -> ChangeBatch {
        let mut changes: Vec<Change> = self
            .changes
            .drain()
            .map(|(path, kind)| Change { path, kind })
            .collect();
        changes.sort_by(|a, b| a.path.cmp(&b.path));
        let rescan = self.rescan;
        self.rescan = false;
        self.first_event = None;
        self.last_event = None;
        ChangeBatch { changes, rescan }
    }
}

/// Collapse two sequential events on the same path into at most one change.
fn merge(old: ChangeKind, new: ChangeKind) -> Option<ChangeKind> {
    use ChangeKind::*;
    match (old, new) {
        // Created then removed within one window would suggest the file never
        // observably existed — but backends that coalesce flags (FSEvents) can
        // report a pre-existing file this way. Keep Removed; the backend's
        // on-disk reconciliation corrects it to Modified if the file exists.
        (Created, Removed) => Some(Removed),
        // Still a brand-new file, whatever happened after.
        (Created, _) => Some(Created),
        // Removed then recreated: contents changed.
        (Removed, Created) => Some(Modified),
        (_, new) => Some(new),
    }
}

#[cfg(test)]
mod merge_tests {
    use super::*;
    use ChangeKind::*;

    #[test]
    fn merge_rules() {
        assert_eq!(merge(Created, Removed), Some(Removed));
        assert_eq!(merge(Created, Modified), Some(Created));
        assert_eq!(merge(Removed, Created), Some(Modified));
        assert_eq!(merge(Modified, Removed), Some(Removed));
        assert_eq!(merge(Modified, Modified), Some(Modified));
        assert_eq!(merge(Removed, Removed), Some(Removed));
    }

    #[test]
    fn coalesces_per_path() {
        let mut p = Pending::default();
        p.record(PathBuf::from("/a"), Created);
        p.record(PathBuf::from("/a"), Modified);
        p.record(PathBuf::from("/b"), Modified);
        p.record(PathBuf::from("/b"), Removed);
        let batch = p.try_take(Duration::ZERO, Duration::ZERO).unwrap();
        assert_eq!(
            batch.changes,
            vec![
                Change {
                    path: PathBuf::from("/a"),
                    kind: Created
                },
                Change {
                    path: PathBuf::from("/b"),
                    kind: Removed
                },
            ]
        );
        assert!(!batch.rescan);
    }

    #[test]
    fn empty_pending_yields_nothing() {
        let mut p = Pending::default();
        assert!(p.try_take(Duration::ZERO, Duration::ZERO).is_none());
    }

    #[test]
    fn created_then_removed_survives_as_removed() {
        // FSEvents can coalesce flags so a pre-existing file reports as
        // Created then Removed; dropping the pair would lose a real delete.
        let mut p = Pending::default();
        p.record(PathBuf::from("/c"), Created);
        p.record(PathBuf::from("/c"), Removed);
        let batch = p.try_take(Duration::ZERO, Duration::ZERO).unwrap();
        assert_eq!(
            batch.changes,
            vec![Change {
                path: PathBuf::from("/c"),
                kind: Removed
            }]
        );
    }

    #[test]
    fn debounce_holds_until_quiet() {
        let mut p = Pending::default();
        p.record(PathBuf::from("/a"), Modified);
        assert!(
            p.try_take(Duration::from_secs(60), Duration::from_secs(120))
                .is_none()
        );
        assert!(
            p.try_take(Duration::ZERO, Duration::from_secs(120))
                .is_some()
        );
    }

    #[test]
    fn rescan_released_alone() {
        let mut p = Pending::default();
        p.record_rescan();
        let batch = p.try_take(Duration::ZERO, Duration::ZERO).unwrap();
        assert!(batch.rescan);
        assert!(batch.changes.is_empty());
        assert!(p.try_take(Duration::ZERO, Duration::ZERO).is_none());
    }
}
