//! Embedded built-in task tree extraction.
//!
//! Release builds embed the `src/builtins/aspect` tree via `include_dir!`
//! and extract it to disk on first use so the AXL runtime can `load(...)`
//! files by path. Debug builds skip extraction and point straight at the
//! source tree.
//!
//! Extraction is crash-safe: writes go to a temp directory and are
//! published with an atomic `rename`, gated by a `.complete` marker
//! written last. See [`extract_aspect_builtins`] for the full protocol.

#[cfg(any(not(debug_assertions), test))]
use std::path::Path;
use std::path::PathBuf;

#[cfg(not(debug_assertions))]
use include_dir::{Dir, DirEntry, include_dir};

#[cfg(not(debug_assertions))]
static ASPECT_DIR: Dir<'_> = include_dir!("$CARGO_MANIFEST_DIR/src/builtins/aspect");

#[cfg(debug_assertions)]
pub fn expand_builtins(
    _root_dir: PathBuf,
    _broot: PathBuf,
) -> std::io::Result<Vec<(String, PathBuf)>> {
    let manifest_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    Ok(vec![(
        "aspect".to_string(),
        manifest_dir.join("src/builtins/aspect"),
    )])
}

#[cfg(not(debug_assertions))]
pub fn expand_builtins(
    _root_dir: PathBuf,
    broot: PathBuf,
) -> std::io::Result<Vec<(String, PathBuf)>> {
    let mut combined = String::new();
    let mut files: Vec<(PathBuf, &[u8])> = Vec::new();
    for entry in ASPECT_DIR.find("**/*").unwrap() {
        if let DirEntry::File(f) = entry {
            combined.push_str(f.path().to_str().unwrap());
            combined.push_str(f.contents_utf8().unwrap_or(""));
            files.push((f.path().to_path_buf(), f.contents()));
        }
    }
    let content_hash = sha256::digest(combined);
    let aspect_dir = extract_aspect_builtins(&broot, &content_hash, &files)?;
    Ok(vec![("aspect".to_string(), aspect_dir)])
}

/// Marker written into `{content_hash}/` after every file is flushed.
/// Its presence is the sole signal that extraction is complete.
#[cfg(any(not(debug_assertions), test))]
const COMPLETE_MARKER: &str = ".complete";

/// Atomically extract `files` into `{broot}/{content_hash}/aspect/` and
/// return that path.
///
/// Protocol:
///   1. `{content_hash}/.complete` present → reuse, no work.
///   2. Otherwise, write to a unique sibling temp dir, write
///      `.complete`, then `rename` it into place. Any pre-existing
///      partial `{content_hash}/` (no marker — left by a crashed run
///      or an older CLI version) is first moved aside via rename to a
///      unique trash path, then removed.
///
/// Concurrent invocations race on `rename(2)`: the winner publishes,
/// losers observe the winner's `.complete` and drop their temp dirs.
/// Using rename (not `remove_dir_all`) for both the stale-cleanup and
/// the publish step keeps every transition atomic, so a loser can
/// never delete a winner's just-published tree.
#[cfg(any(not(debug_assertions), test))]
fn extract_aspect_builtins(
    broot: &Path,
    content_hash: &str,
    files: &[(PathBuf, &[u8])],
) -> std::io::Result<PathBuf> {
    use std::fs;

    let final_dir = broot.join(content_hash);
    let aspect_dir = final_dir.join("aspect");

    if final_dir.join(COMPLETE_MARKER).exists() {
        return Ok(aspect_dir);
    }

    fs::create_dir_all(broot)?;

    let tmp_dir = broot.join(format!("{}.tmp.{}", content_hash, uuid::Uuid::new_v4()));
    let result = write_and_publish(&tmp_dir, &final_dir, files);
    // Clean up the temp dir whenever it still exists — error paths and
    // the loser-adopts-winner path both leave it behind. The success-
    // rename path moves tmp_dir into final_dir, so remove is a no-op.
    let _ = fs::remove_dir_all(&tmp_dir);
    result.map(|_| aspect_dir)
}

/// Write `files` into `tmp_dir/aspect/`, mark complete, and rename into
/// `final_dir`. Splits the body of [`extract_aspect_builtins`] so the
/// caller can centralize tmp-dir cleanup on any failure.
#[cfg(any(not(debug_assertions), test))]
fn write_and_publish(
    tmp_dir: &Path,
    final_dir: &Path,
    files: &[(PathBuf, &[u8])],
) -> std::io::Result<()> {
    use std::fs;

    let tmp_aspect = tmp_dir.join("aspect");
    for (rel_path, contents) in files {
        let out_path = tmp_aspect.join(rel_path);
        fs::create_dir_all(out_path.parent().unwrap())?;
        fs::write(&out_path, contents)?;
    }
    fs::write(tmp_dir.join(COMPLETE_MARKER), "")?;

    evict_stale_dir(final_dir);

    match fs::rename(tmp_dir, final_dir) {
        Ok(_) => Ok(()),
        // Concurrent winner published first — adopt their copy.
        Err(_) if final_dir.join(COMPLETE_MARKER).exists() => Ok(()),
        Err(e) => Err(e),
    }
}

/// Move a stale partial `final_dir` aside (rename-to-trash, then
/// remove) so the subsequent publish rename has a clear target. No-op
/// when `final_dir` doesn't exist or already carries `.complete`.
///
/// Best-effort throughout: a concurrent thread may have already
/// trashed the same dir, or our trash-remove may race a sibling's
/// remove. The post-rename marker check is the authoritative outcome.
#[cfg(any(not(debug_assertions), test))]
fn evict_stale_dir(final_dir: &Path) {
    use std::fs;

    if !final_dir.exists() || final_dir.join(COMPLETE_MARKER).exists() {
        return;
    }
    let Some(parent) = final_dir.parent() else {
        return;
    };
    let name = final_dir.file_name().and_then(|s| s.to_str()).unwrap_or("");
    let trash = parent.join(format!("{}.trash.{}", name, uuid::Uuid::new_v4()));
    let _ = fs::rename(final_dir, &trash);
    let _ = fs::remove_dir_all(&trash);
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    fn sample_files() -> Vec<(PathBuf, &'static [u8])> {
        vec![
            (
                PathBuf::from("feature/github_status_checks.axl"),
                b"load(\"../lib/artifacts.axl\", \"ArtifactsTrait\")\n" as &[u8],
            ),
            (
                PathBuf::from("lib/artifacts.axl"),
                b"artifacts = struct()\n",
            ),
            (PathBuf::from("format.axl"), b"format = task()\n"),
        ]
    }

    fn assert_full_tree(aspect_dir: &Path) {
        assert!(aspect_dir.join("feature/github_status_checks.axl").exists());
        assert!(aspect_dir.join("lib/artifacts.axl").exists());
        assert!(aspect_dir.join("format.axl").exists());
    }

    fn assert_no_debris(broot: &Path) {
        for entry in fs::read_dir(broot).unwrap() {
            let name = entry.unwrap().file_name().into_string().unwrap();
            assert!(
                !name.contains(".tmp.") && !name.contains(".trash."),
                "leftover debris: {name}"
            );
        }
    }

    #[test]
    fn cold_extract_writes_full_tree_and_marker() {
        let tmp = tempfile::tempdir().unwrap();
        let aspect_dir = extract_aspect_builtins(tmp.path(), "abc123", &sample_files()).unwrap();
        assert_full_tree(&aspect_dir);
        assert!(tmp.path().join("abc123").join(COMPLETE_MARKER).exists());
        assert_no_debris(tmp.path());
    }

    #[test]
    fn warm_call_with_marker_is_idempotent() {
        let tmp = tempfile::tempdir().unwrap();
        extract_aspect_builtins(tmp.path(), "abc123", &sample_files()).unwrap();
        let mtime_before = fs::metadata(tmp.path().join("abc123/aspect/format.axl"))
            .unwrap()
            .modified()
            .unwrap();
        let aspect_dir = extract_aspect_builtins(tmp.path(), "abc123", &sample_files()).unwrap();
        let mtime_after = fs::metadata(aspect_dir.join("format.axl"))
            .unwrap()
            .modified()
            .unwrap();
        assert_eq!(mtime_before, mtime_after);
    }

    #[test]
    fn partial_extraction_without_marker_is_repaired() {
        let tmp = tempfile::tempdir().unwrap();
        let stale_dir = tmp.path().join("abc123");
        let stale_aspect = stale_dir.join("aspect");
        fs::create_dir_all(stale_aspect.join("feature")).unwrap();
        fs::write(
            stale_aspect.join("feature/github_status_checks.axl"),
            b"stale partial content",
        )
        .unwrap();
        assert!(!stale_dir.join(COMPLETE_MARKER).exists());

        let aspect_dir = extract_aspect_builtins(tmp.path(), "abc123", &sample_files()).unwrap();
        assert_full_tree(&aspect_dir);
        assert!(tmp.path().join("abc123").join(COMPLETE_MARKER).exists());
        assert_eq!(
            fs::read(aspect_dir.join("feature/github_status_checks.axl")).unwrap(),
            b"load(\"../lib/artifacts.axl\", \"ArtifactsTrait\")\n"
        );
        assert_no_debris(tmp.path());
    }

    #[test]
    fn extraction_is_concurrency_safe() {
        use std::sync::Arc;
        use std::thread;

        let tmp = Arc::new(tempfile::tempdir().unwrap());
        let files = Arc::new(sample_files());

        let handles: Vec<_> = (0..8)
            .map(|_| {
                let tmp = Arc::clone(&tmp);
                let files = Arc::clone(&files);
                thread::spawn(move || {
                    extract_aspect_builtins(tmp.path(), "abc123", &files).unwrap()
                })
            })
            .collect();
        for h in handles {
            assert_full_tree(&h.join().unwrap());
        }
        assert!(tmp.path().join("abc123").join(COMPLETE_MARKER).exists());
        assert_no_debris(tmp.path());
    }

    #[test]
    fn evict_stale_dir_skips_complete_dir() {
        let tmp = tempfile::tempdir().unwrap();
        let final_dir = tmp.path().join("abc123");
        fs::create_dir_all(final_dir.join("aspect")).unwrap();
        fs::write(final_dir.join(COMPLETE_MARKER), "").unwrap();
        fs::write(final_dir.join("aspect/keepme"), b"x").unwrap();

        evict_stale_dir(&final_dir);

        assert!(final_dir.join("aspect/keepme").exists());
    }

    #[test]
    fn evict_stale_dir_removes_partial_dir() {
        let tmp = tempfile::tempdir().unwrap();
        let final_dir = tmp.path().join("abc123");
        fs::create_dir_all(final_dir.join("aspect/feature")).unwrap();

        evict_stale_dir(&final_dir);

        assert!(!final_dir.exists());
        assert_no_debris(tmp.path());
    }

    #[test]
    fn evict_stale_dir_no_op_when_absent() {
        let tmp = tempfile::tempdir().unwrap();
        evict_stale_dir(&tmp.path().join("never-existed"));
    }
}
