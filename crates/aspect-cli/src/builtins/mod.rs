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
                b"load(\"../private/lib/artifacts.axl\", \"ArtifactsTrait\")\n" as &[u8],
            ),
            (
                PathBuf::from("private/lib/artifacts.axl"),
                b"artifacts = struct()\n",
            ),
            (PathBuf::from("format.axl"), b"format = task()\n"),
        ]
    }

    fn assert_full_tree(aspect_dir: &Path) {
        assert!(aspect_dir.join("feature/github_status_checks.axl").exists());
        assert!(aspect_dir.join("private/lib/artifacts.axl").exists());
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
            b"load(\"../private/lib/artifacts.axl\", \"ArtifactsTrait\")\n"
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

    /// Normalize a POSIX-ish path (resolve `.`/`..`, drop redundant `/`)
    /// without touching the filesystem. Used to resolve a relative load
    /// against the loading file's directory.
    fn norm(path: &str) -> String {
        let mut out: Vec<&str> = Vec::new();
        for seg in path.split('/') {
            match seg {
                "" | "." => {}
                ".." => {
                    if matches!(out.last(), Some(&s) if s != "..") {
                        out.pop();
                    } else {
                        out.push("..");
                    }
                }
                s => out.push(s),
            }
        }
        out.join("/")
    }

    /// Within the @aspect builtin tree, every load must use its canonical
    /// *form*, which encodes the public/private boundary in the path syntax:
    ///
    ///   - a load whose target resolves OUTSIDE `private/` (a public
    ///     task/facade or `feature/*`) is fully-qualified `@aspect//…`;
    ///   - a load whose target resolves UNDER `private/` is RELATIVE
    ///     (`./…` / `../…`) from the loading file.
    ///
    /// External namespaces (`@bazel//`, `@std//`, …) are always public and
    /// left as-is. This makes buildifier's canonical single-block sort group
    /// public (`@`-prefixed) loads above private (`.`-prefixed) ones with no
    /// manual blocks. The guard fails on any load written in the wrong form.
    ///
    /// The tree is read from a compile-time `include_dir!` embed rather than
    /// the filesystem so the test passes under Bazel's sandbox (where the
    /// cargo workspace layout isn't on disk).
    #[test]
    fn loads_use_canonical_public_private_form() {
        use include_dir::{Dir, include_dir};
        static ASPECT_DIR: Dir<'_> = include_dir!("$CARGO_MANIFEST_DIR/src/builtins/aspect");

        // Real load(...) statements only — skip module docstrings (which may
        // carry illustrative load() examples). A load line is one whose first
        // non-space token is `load(` or that is a `"…"` string immediately
        // following a `load(` opener; we approximate by scanning lines that
        // start with `load("` plus the multi-line opener `load(` then a quoted
        // path on the next non-blank line.
        let load_re = regex_lite_load();

        let mut violations = Vec::new();
        for f in ASPECT_DIR.find("**/*.axl").unwrap() {
            let rel = f.path();
            let file_dir = rel
                .parent()
                .map(|p| p.to_string_lossy().to_string())
                .unwrap_or_default();
            let Some(contents) = f.as_file().and_then(|file| file.contents_utf8()) else {
                continue;
            };
            for path in load_re(contents) {
                let want = canonical_form(&path, &file_dir);
                if let Some(want) = want {
                    if want != path {
                        violations.push(format!(
                            "{}: load(\"{}\") should be load(\"{}\")",
                            rel.display(),
                            path,
                            want
                        ));
                    }
                }
            }
        }
        assert!(
            violations.is_empty(),
            "@aspect builtin loads must use canonical form (public → @aspect//…, \
             private → relative ./ or ../). Offenders:\n  {}",
            violations.join("\n  ")
        );
    }

    /// Resolve a load path to its module-relative target and return the
    /// canonical load string, or None for external namespaces (left as-is)
    /// or unparseable input.
    fn canonical_form(load_path: &str, file_dir: &str) -> Option<String> {
        let target = if let Some(sub) = load_path.strip_prefix("@aspect//") {
            norm(sub.trim_start_matches(':'))
        } else if load_path.starts_with('@') {
            return None; // external namespace
        } else {
            // relative: resolve against the loading file's dir
            let joined = if file_dir.is_empty() {
                load_path.to_string()
            } else {
                format!("{file_dir}/{load_path}")
            };
            norm(&joined)
        };
        let under_private = target == "private" || target.starts_with("private/");
        if !under_private {
            return Some(format!("@aspect//{target}"));
        }
        // relative form from file_dir to target
        Some(rel_from(file_dir, &target))
    }

    /// Compute a relative `./`/`../` path from `from_dir` to `to` (both
    /// module-relative, normalized). Mirrors posixpath.relpath + ensuring a
    /// leading `./` for same-or-descendant targets.
    fn rel_from(from_dir: &str, to: &str) -> String {
        let from: Vec<&str> = if from_dir.is_empty() {
            Vec::new()
        } else {
            from_dir.split('/').collect()
        };
        let to_parts: Vec<&str> = to.split('/').collect();
        let mut i = 0;
        while i < from.len() && i < to_parts.len() && from[i] == to_parts[i] {
            i += 1;
        }
        let mut rel: Vec<String> = Vec::new();
        for _ in i..from.len() {
            rel.push("..".to_string());
        }
        for seg in &to_parts[i..] {
            rel.push(seg.to_string());
        }
        let joined = rel.join("/");
        if joined.starts_with("..") {
            joined
        } else {
            format!("./{joined}")
        }
    }

    /// Tiny load-path extractor: returns the first quoted string of every
    /// `load(...)` after a module docstring. Avoids a regex-crate dep.
    fn regex_lite_load() -> impl Fn(&str) -> Vec<String> {
        |contents: &str| {
            let mut out = Vec::new();
            let lines: Vec<&str> = contents.lines().collect();
            let mut i = 0;
            // skip leading blank lines
            while i < lines.len() && lines[i].trim().is_empty() {
                i += 1;
            }
            // skip a module docstring
            if i < lines.len() {
                let s = lines[i].trim_start();
                for q in ["\"\"\"", "'''"] {
                    if let Some(rest) = s.strip_prefix(q) {
                        if rest.contains(q) {
                            i += 1;
                        } else {
                            i += 1;
                            while i < lines.len() && !lines[i].contains(q) {
                                i += 1;
                            }
                            i += 1;
                        }
                        break;
                    }
                }
            }
            while i < lines.len() {
                let trimmed = lines[i].trim_start();
                if trimmed.starts_with("load(") {
                    // path is the first "..." on this or the next non-blank line
                    let mut j = i;
                    let mut path: Option<String> = None;
                    while j < lines.len() && j <= i + 1 {
                        if let Some(start) = lines[j].find('"') {
                            if let Some(end) = lines[j][start + 1..].find('"') {
                                path = Some(lines[j][start + 1..start + 1 + end].to_string());
                                break;
                            }
                        }
                        j += 1;
                    }
                    if let Some(p) = path {
                        out.push(p);
                    }
                }
                i += 1;
            }
            out
        }
    }
}
