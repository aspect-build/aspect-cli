use std::path::{Path, PathBuf};

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

// Marker file written into `{content_hash}/` after every file has been
// flushed. Its presence is the sole signal that extraction is complete
// — `{content_hash}/aspect/...` existing on its own means nothing (a
// previous run could have been killed mid-write, leaving a partial
// tree behind).
const COMPLETE_MARKER: &str = ".complete";

/// Atomically extract the embedded built-in tree into
/// `{broot}/{content_hash}/aspect/` and return that path.
///
/// Extraction strategy:
///   1. If `{broot}/{content_hash}/.complete` exists → reuse as-is.
///   2. Otherwise: nuke any leftover `{broot}/{content_hash}/` (from a
///      previous crashed run or an older CLI that didn't write the
///      marker), write all files into a sibling temp dir, write
///      `.complete`, then `rename` the temp dir into place.
///
/// The rename is atomic on a single filesystem, so concurrent CLI
/// invocations can race safely: the loser observes `ENOTEMPTY` /
/// `EEXIST`, sees the winner's `.complete`, and drops its temp dir.
///
/// Failure mode this prevents: process A creates `{content_hash}/`
/// during the first `fs::create_dir_all(parent)`, gets killed before
/// writing all files. Process B's `aspect_dir.exists()` returns true,
/// it skips extraction, and downstream `load("../lib/foo.axl")` fails
/// with ENOENT. Reported by a CI customer running on an ephemeral
/// cache mount.
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

    // Per-call temp dir avoids collisions between concurrent invocations
    // racing on the same content hash, including in-process concurrency
    // (multiple threads in tests, or future callers). The `rename` below
    // is the cross-process serialization point.
    let tmp_dir = broot.join(format!("{}.tmp.{}", content_hash, uuid::Uuid::new_v4()));
    let tmp_aspect = tmp_dir.join("aspect");

    for (rel_path, contents) in files {
        let out_path = tmp_aspect.join(rel_path);
        fs::create_dir_all(out_path.parent().unwrap())?;
        fs::write(&out_path, contents)?;
    }

    // Marker written LAST: a crash anywhere above leaves the temp dir
    // without `.complete`, and the next run treats it as garbage.
    fs::write(tmp_dir.join(COMPLETE_MARKER), "")?;

    // Two cleanup scenarios at publish time:
    //   - final_dir doesn't exist: rename publishes our tree.
    //   - final_dir exists without `.complete` (stale partial from a
    //     prior crashed run, or older CLI version): atomically move it
    //     aside, then publish our tree.
    //
    // The "move aside" is itself a rename to a unique trash path so
    // concurrent threads / processes can't race on a non-atomic
    // remove_dir_all + rename sequence (where T1's remove would delete
    // T2's just-published work). If the trash-rename loses to a
    // concurrent winner, we fall through and pick up their `.complete`.
    if final_dir.exists() && !final_dir.join(COMPLETE_MARKER).exists() {
        let trash = broot.join(format!("{}.trash.{}", content_hash, uuid::Uuid::new_v4()));
        // Best-effort: rename can fail if someone else moved it first.
        // That's fine — they'll either publish a complete copy or fail
        // outright, and we'll observe the outcome at our own rename.
        let _ = fs::rename(&final_dir, &trash);
        let _ = fs::remove_dir_all(&trash);
    }

    match fs::rename(&tmp_dir, &final_dir) {
        Ok(_) => Ok(aspect_dir),
        Err(_) if final_dir.join(COMPLETE_MARKER).exists() => {
            // Someone else won the race and published a complete copy.
            // Drop our temp dir and use theirs.
            let _ = fs::remove_dir_all(&tmp_dir);
            Ok(aspect_dir)
        }
        Err(e) => {
            let _ = fs::remove_dir_all(&tmp_dir);
            Err(e)
        }
    }
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
            (PathBuf::from("lib/artifacts.axl"), b"artifacts = struct()\n"),
            (PathBuf::from("format.axl"), b"format = task()\n"),
        ]
    }

    fn assert_full_tree(aspect_dir: &Path) {
        assert!(aspect_dir.join("feature/github_status_checks.axl").exists());
        assert!(aspect_dir.join("lib/artifacts.axl").exists());
        assert!(aspect_dir.join("format.axl").exists());
    }

    #[test]
    fn cold_extract_writes_full_tree_and_marker() {
        let tmp = tempfile::tempdir().unwrap();
        let aspect_dir =
            extract_aspect_builtins(tmp.path(), "abc123", &sample_files()).unwrap();
        assert_full_tree(&aspect_dir);
        assert!(tmp.path().join("abc123").join(COMPLETE_MARKER).exists());
    }

    #[test]
    fn warm_call_with_marker_is_idempotent() {
        let tmp = tempfile::tempdir().unwrap();
        extract_aspect_builtins(tmp.path(), "abc123", &sample_files()).unwrap();
        let mtime_before = fs::metadata(tmp.path().join("abc123/aspect/format.axl"))
            .unwrap()
            .modified()
            .unwrap();
        // Second call must not rewrite — we trust the marker.
        let aspect_dir =
            extract_aspect_builtins(tmp.path(), "abc123", &sample_files()).unwrap();
        let mtime_after = fs::metadata(aspect_dir.join("format.axl"))
            .unwrap()
            .modified()
            .unwrap();
        assert_eq!(mtime_before, mtime_after);
    }

    #[test]
    fn partial_extraction_without_marker_is_repaired() {
        // Simulates the reported failure: a previous run crashed after
        // writing one nested file (and `create_dir_all` materialized the
        // parent directories), leaving `lib/artifacts.axl` missing.
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
        assert!(!stale_aspect.join("lib/artifacts.axl").exists());

        let aspect_dir =
            extract_aspect_builtins(tmp.path(), "abc123", &sample_files()).unwrap();
        assert_full_tree(&aspect_dir);
        assert!(tmp.path().join("abc123").join(COMPLETE_MARKER).exists());

        // Stale content should be overwritten with the real payload.
        let bytes =
            fs::read(aspect_dir.join("feature/github_status_checks.axl")).unwrap();
        assert_eq!(
            bytes,
            b"load(\"../lib/artifacts.axl\", \"ArtifactsTrait\")\n"
        );
    }

    #[test]
    fn extraction_is_concurrency_safe() {
        // Two threads racing on the same content hash must both observe
        // a complete tree, with no partial dirs left behind.
        use std::sync::Arc;
        use std::thread;

        let tmp = Arc::new(tempfile::tempdir().unwrap());
        let files = Arc::new(sample_files());

        let handles: Vec<_> = (0..4)
            .map(|_| {
                let tmp = Arc::clone(&tmp);
                let files = Arc::clone(&files);
                thread::spawn(move || {
                    extract_aspect_builtins(tmp.path(), "abc123", &files).unwrap()
                })
            })
            .collect();
        for h in handles {
            let aspect_dir = h.join().unwrap();
            assert_full_tree(&aspect_dir);
        }
        assert!(tmp.path().join("abc123").join(COMPLETE_MARKER).exists());

        // No leftover `<hash>.tmp.<pid>` debris.
        for entry in fs::read_dir(tmp.path()).unwrap() {
            let name = entry.unwrap().file_name().into_string().unwrap();
            assert!(!name.contains(".tmp."), "leftover temp dir: {name}");
        }
    }
}
