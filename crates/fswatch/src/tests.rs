use std::fs;
use std::path::PathBuf;
use std::time::{Duration, Instant};

use crate::{Backend, ChangeBatch, ChangeKind, Subscription, WatchConfig, watch};

fn quick_config(root: PathBuf) -> WatchConfig {
    let mut config = WatchConfig::new(vec![root]);
    config.debounce = Duration::from_millis(50);
    config
}

fn recv_batch(sub: &mut Box<dyn Subscription>) -> ChangeBatch {
    let deadline = Instant::now() + Duration::from_secs(10);
    loop {
        if let Some(batch) = sub.try_recv().unwrap() {
            return batch;
        }
        assert!(Instant::now() < deadline, "no batch within deadline");
        std::thread::sleep(Duration::from_millis(20));
    }
}

// Watch backends need a moment after `watch()` returns before events flow, and
// FSEvents replays just-past events (e.g. the tempdir's own creation); wait for
// the stream to come up, then discard that noise.
fn settle(sub: &mut Box<dyn Subscription>) {
    std::thread::sleep(Duration::from_millis(500));
    sub.drain();
}

fn watchman_available() -> bool {
    std::process::Command::new("watchman")
        .arg("version")
        .output()
        .is_ok_and(|out| out.status.success())
}

/// Skip watchman tests (with a note) on machines without the binary.
macro_rules! backend_tests {
    ($($name:ident),* $(,)?) => {
        mod notify {
            $(#[test]
            fn $name() {
                super::$name(crate::Backend::Notify);
            })*
        }
        mod watchman {
            $(#[test]
            fn $name() {
                if !super::watchman_available() {
                    eprintln!("watchman not on PATH; skipping");
                    return;
                }
                super::$name(crate::Backend::Watchman);
            })*
        }
    };
}

backend_tests!(
    reports_created_file,
    reports_removed_file,
    ignores_configured_prefixes,
    drain_discards_pending,
);

fn reports_created_file(backend: Backend) {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().canonicalize().unwrap();
    let mut sub = watch(backend, quick_config(root.clone())).unwrap();
    settle(&mut sub);
    fs::write(root.join("a.txt"), "hello").unwrap();
    let batch = recv_batch(&mut sub);
    assert!(
        batch.changes.iter().any(|c| c.path == root.join("a.txt")),
        "missing a.txt in {batch:?}"
    );
}

fn reports_removed_file(backend: Backend) {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().canonicalize().unwrap();
    fs::write(root.join("a.txt"), "hello").unwrap();
    let mut sub = watch(backend, quick_config(root.clone())).unwrap();
    settle(&mut sub);
    fs::remove_file(root.join("a.txt")).unwrap();
    let batch = recv_batch(&mut sub);
    let change = batch
        .changes
        .iter()
        .find(|c| c.path == root.join("a.txt"))
        .unwrap_or_else(|| panic!("missing a.txt in {batch:?}"));
    assert_eq!(change.kind, ChangeKind::Removed);
}

fn ignores_configured_prefixes(backend: Backend) {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().canonicalize().unwrap();
    fs::create_dir(root.join("ignored")).unwrap();
    let mut config = quick_config(root.clone());
    config.ignore_prefixes = vec![root.join("ignored")];
    let mut sub = watch(backend, config).unwrap();
    settle(&mut sub);
    fs::write(root.join("ignored/a.txt"), "x").unwrap();
    fs::write(root.join("kept.txt"), "x").unwrap();
    let batch = recv_batch(&mut sub);
    assert!(
        batch
            .changes
            .iter()
            .any(|c| c.path == root.join("kept.txt")),
        "{batch:?}"
    );
    assert!(
        !batch
            .changes
            .iter()
            .any(|c| c.path.starts_with(root.join("ignored"))),
        "{batch:?}"
    );
}

#[test]
fn auto_backend_detection_follows_watchmanconfig() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().canonicalize().unwrap();
    let nested = root.join("a/b");
    fs::create_dir_all(&nested).unwrap();
    assert!(!crate::in_watchman_project(&root));
    fs::write(root.join(".watchmanconfig"), "{}").unwrap();
    assert!(crate::in_watchman_project(&root));
    // Only the watched root itself opts in — no ancestor discovery.
    assert!(!crate::in_watchman_project(&nested));
}

fn drain_discards_pending(backend: Backend) {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().canonicalize().unwrap();
    let mut sub = watch(backend, quick_config(root.clone())).unwrap();
    settle(&mut sub);
    fs::write(root.join("noise.txt"), "x").unwrap();
    // Wait until the write is definitely pending, then discard it.
    std::thread::sleep(Duration::from_millis(500));
    sub.drain();
    assert!(sub.try_recv().unwrap().is_none());
    fs::write(root.join("signal.txt"), "x").unwrap();
    let batch = recv_batch(&mut sub);
    assert!(
        batch
            .changes
            .iter()
            .any(|c| c.path == root.join("signal.txt")),
        "{batch:?}"
    );
}

#[test]
fn new_top_level_directory_gets_watched() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().canonicalize().unwrap();
    let mut sub = watch(Backend::default(), quick_config(root.clone())).unwrap();
    settle(&mut sub);
    fs::create_dir(root.join("newdir")).unwrap();
    // Drain until the creation lands and the recursive watch is registered.
    let deadline = Instant::now() + Duration::from_secs(10);
    loop {
        if let Some(batch) = sub.try_recv().unwrap() {
            if batch.changes.iter().any(|c| c.path == root.join("newdir")) {
                break;
            }
        }
        assert!(Instant::now() < deadline, "newdir creation never observed");
        std::thread::sleep(Duration::from_millis(20));
    }
    fs::write(root.join("newdir/inner.txt"), "x").unwrap();
    let batch = recv_batch(&mut sub);
    assert!(
        batch
            .changes
            .iter()
            .any(|c| c.path == root.join("newdir/inner.txt")),
        "missing inner.txt in {batch:?}"
    );
}
