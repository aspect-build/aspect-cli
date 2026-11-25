//! Writing the user's `~/.aspect/config.json`, and the `tips` object of it
//! and of a repository's `.aspect/config.json`.
//!
//! [`update`] is the user's file's one writer (`aspect auth` and the tips
//! silences both go through it). It runs a read-modify-write while holding an
//! exclusive `flock` of a lock file beside the config as named
//! (`.config.json.lock` in `~/.aspect`, also when the config is a symlink),
//! so concurrent CLIs never lose each other's changes. Where no lock can be
//! taken (some NFS and FUSE homes) it writes without one, with a warning once
//! when that write succeeds; on Windows it takes none. It writes a temp file
//! beside the config and renames it over, then syncs the directory, keeping
//! the file's mode and every key the change does not touch. The JSON is
//! rewritten with two-space indents, the keys in their order. A symlinked
//! file, dangling or not, is written through to its target. A file that is
//! read-only, owned by another user (the rename would replace it with one of
//! this user's) or not a JSON object is refused; an empty file is `{}`.
//!
//! `tips` is an object, so it can grow: `{"tips": {"silence": ["<id>", ...]}}`
//! silences those tips. Both the user's file and a repository's may set it,
//! and their lists join a task's `--tips:silence` and its `config.axl`'s
//! ([`config_silence`]); the writer changes the user's only.

use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};

use serde_json::{Map, Value};

/// Apply `change` to the top-level object of the `config.json` at `path` and
/// write it back when `change` returns true (the file changed), all under the
/// file's lock (see the module docs). Creates the directory and the file when
/// missing. Fails, writing nothing, when the file is read-only, another
/// user's or not a JSON object, since its other keys could not be kept, or
/// when `change` fails. Returns what `change` did.
pub(crate) fn update(
    path: &Path,
    change: impl FnOnce(&mut Map<String, Value>) -> anyhow::Result<bool>,
) -> anyhow::Result<bool> {
    update_warning(path, change, |w| {
        if crate::config_json::first_time(w) {
            crate::diag::warn(w)
        }
    })
}

/// [`update`], handing `warn` what it would warn about: only that it wrote
/// without a lock, and only once the write succeeded, so a write that fails
/// says so alone.
fn update_warning(
    path: &Path,
    change: impl FnOnce(&mut Map<String, Value>) -> anyhow::Result<bool>,
    warn: impl FnOnce(&str),
) -> anyhow::Result<bool> {
    let target = target(path);
    for dir in [path.parent(), target.parent()].into_iter().flatten() {
        if !dir.as_os_str().is_empty() {
            fs::create_dir_all(dir)
                .map_err(|e| anyhow::anyhow!("failed to create {}: {e}", dir.display()))?;
        }
    }
    let (_lock, unlocked) = match lock(path) {
        Ok(lock) => (Some(lock), None),
        Err(e) => (None, Some(e)),
    };
    if let Ok(meta) = fs::metadata(&target) {
        refuse(&target, &meta, None)?;
    }
    let mut object = crate::config_json::object(&target)
        .map_err(|e| anyhow::anyhow!("{e}; not writing it"))?
        .unwrap_or_default();
    if !change(&mut object)? {
        return Ok(false);
    }
    let text = serde_json::to_string_pretty(&object)
        .map_err(|e| anyhow::anyhow!("failed to serialize {}: {e}", target.display()))?
        + "\n";
    replace(&target, text.as_bytes())?;
    if let Some(e) = unlocked {
        warn(&format!(
            "cannot lock {} ({e}), so {} was written without a lock; a concurrent change may be lost",
            lock_path(path).display(),
            target.display()
        ));
    }
    Ok(true)
}

/// Why the existing file at `path` (`meta`) must not be replaced by a file
/// owned by `uid` (`None` to skip that check): it is read-only, or another
/// user's.
fn refuse(path: &Path, meta: &fs::Metadata, uid: Option<u32>) -> anyhow::Result<()> {
    if meta.permissions().readonly() {
        anyhow::bail!("{} is read-only; not writing it", path.display());
    }
    #[cfg(unix)]
    if let Some(uid) = uid {
        let owner = std::os::unix::fs::MetadataExt::uid(meta);
        if owner != uid {
            anyhow::bail!(
                "{} is owned by another user (uid {owner}); not writing it",
                path.display()
            );
        }
    }
    let _ = uid;
    Ok(())
}

/// The file `path` names: itself, or, through each symlink, what it points
/// at, whether or not that exists.
fn target(path: &Path) -> PathBuf {
    let mut path = path.to_path_buf();
    for _ in 0..40 {
        let Ok(link) = fs::read_link(&path) else {
            break;
        };
        path = match path.parent() {
            Some(parent) => parent.join(link),
            None => link,
        };
    }
    path
}

/// The lock file of the config at `path`: `.<name>.lock` beside it, as
/// named rather than as resolved, so a config linked into a dotfiles
/// checkout leaves nothing there.
fn lock_path(path: &Path) -> PathBuf {
    let name = path.file_name().unwrap_or_default().to_string_lossy();
    path.with_file_name(format!(".{name}.lock"))
}

/// An exclusive advisory `flock` of [`lock_path`], opened read-write (an NFS
/// lock needs a writable descriptor) and created readable by this user
/// alone; released on drop. Else why none can be taken.
#[cfg(unix)]
fn lock(path: &Path) -> Result<nix::fcntl::Flock<fs::File>, String> {
    use std::os::unix::fs::OpenOptionsExt;
    fs::OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .truncate(false)
        .mode(0o600)
        .open(lock_path(path))
        .map_err(|e| e.to_string())
        .and_then(|file| {
            nix::fcntl::Flock::lock(file, nix::fcntl::FlockArg::LockExclusive)
                .map_err(|(_, e)| e.to_string())
        })
}

#[cfg(not(unix))]
fn lock(_path: &Path) -> Result<(), String> {
    Ok(())
}

/// Replace `path` whole with `bytes` through a temp file in its directory,
/// given `path`'s mode when it exists, then sync the directory so the rename
/// is durable. Refuses to replace a file whose owner is not the temp file's
/// ([`refuse`]): this process's user.
fn replace(path: &Path, bytes: &[u8]) -> anyhow::Result<()> {
    let name = path.file_name().unwrap_or_default().to_string_lossy();
    let temp = path.with_file_name(format!(".{name}.{}.tmp", uuid::Uuid::new_v4()));
    let failed = |e: std::io::Error| anyhow::anyhow!("failed to write {}: {e}", path.display());
    let result = (|| {
        let mut file = fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&temp)
            .map_err(failed)?;
        if let Ok(meta) = fs::metadata(path) {
            #[cfg(unix)]
            let ours = Some(std::os::unix::fs::MetadataExt::uid(
                &file.metadata().map_err(failed)?,
            ));
            #[cfg(not(unix))]
            let ours = None;
            refuse(path, &meta, ours)?;
            file.set_permissions(meta.permissions()).map_err(failed)?;
        }
        file.write_all(bytes).map_err(failed)?;
        file.sync_all().map_err(failed)?;
        fs::rename(&temp, path).map_err(failed)
    })();
    if result.is_err() {
        let _ = fs::remove_file(&temp);
        return result;
    }
    // Not every filesystem syncs a directory; the file is written either way.
    #[cfg(unix)]
    if let Some(dir) = path.parent() {
        let _ = fs::File::open(dir).and_then(|d| d.sync_all());
    }
    Ok(())
}

/// The user's `config.json`, or `path` when given.
pub(crate) fn path_or_user(path: Option<&str>) -> anyhow::Result<PathBuf> {
    match path {
        Some(path) => Ok(PathBuf::from(path)),
        None => crate::config_json::user_path(),
    }
}

/// The ids in `tips.silence` of the `config.json` at `path`, and a warning
/// naming the file for each entry that is not a tip id ([`valid_id`]), which
/// is left out, the rest kept. Empty when the file or a key is missing, else
/// an error naming the file: the file's own (`crate::config_json::object`),
/// or that `tips` is not an object or `silence` not a list.
pub(crate) fn read_silence(path: &Path) -> Result<(Vec<String>, Vec<String>), String> {
    let Some(object) = crate::config_json::object(path)? else {
        return Ok(Default::default());
    };
    let entries = silence_list(&object).map_err(|e| format!("{} {e}", path.display()))?;
    let mut ids = Vec::new();
    let mut warnings = Vec::new();
    for entry in entries {
        match entry.as_str().filter(|id| valid_id(id)) {
            Some(id) => ids.push(id.to_owned()),
            None => {
                let hint = match entry.as_str() {
                    Some(id) if id.contains(',') => "; split it into list items ([\"x\", \"y\"])",
                    _ => "",
                };
                warnings.push(format!(
                    "{} \"tips\".\"silence\" {entry} is not a tip id, so it silences nothing{hint}",
                    path.display()
                ));
            }
        }
    }
    Ok((ids, warnings))
}

/// The ids [`read_silence`] reads, its warnings dropped.
#[cfg(test)]
pub(crate) fn tips_silence(path: &Path) -> Result<Vec<String>, String> {
    read_silence(path).map(|(ids, _)| ids)
}

/// The tips silenced by the `config.json` files: `tips.silence` of `user`
/// and of `repo`, each empty when its path is `None`, and a warning for each
/// invalid file, whose ids are left out, and for each entry that is not a
/// tip id. A file's own warning is worded as every reader of it words it,
/// so it shows once.
pub(crate) struct ConfigSilence {
    pub user: Vec<String>,
    pub repo: Vec<String>,
    pub warnings: Vec<String>,
}

pub(crate) fn config_silence(user: Option<&Path>, repo: Option<&Path>) -> ConfigSilence {
    let mut warnings = Vec::new();
    let mut read = |path: Option<&Path>| {
        let Some(path) = path else {
            return Vec::new();
        };
        match read_silence(path) {
            Ok((ids, bad)) => {
                warnings.extend(bad);
                ids
            }
            Err(e) => {
                warnings.push(format!("{e}; ignoring it"));
                Vec::new()
            }
        }
    };
    let (user, repo) = (read(user), read(repo));
    ConfigSilence {
        user,
        repo,
        warnings,
    }
}

/// `object`'s `tips.silence` entries, as written, or what is wrong with its
/// shape: `tips` not an object, or `silence` not a list.
fn silence_list(object: &Map<String, Value>) -> Result<Vec<Value>, String> {
    let Some(tips) = object.get("tips") else {
        return Ok(Vec::new());
    };
    let Value::Object(tips) = tips else {
        return Err(format!("\"tips\" must be an object, got {tips}"));
    };
    match tips.get("silence") {
        None => Ok(Vec::new()),
        Some(Value::Array(entries)) => Ok(entries.clone()),
        Some(other) => Err(format!(
            "\"tips\".\"silence\" must be a list of tip ids, got {other}"
        )),
    }
}

/// Whether `id` can be a tip's id: not empty, and no whitespace or comma
/// (`--tips:silence=a,b` is one id, so a comma there is always a mistake).
pub(crate) fn valid_id(id: &str) -> bool {
    !id.is_empty() && !id.chars().any(|c| c.is_whitespace() || c == ',')
}

/// Add `ids` to (`silence`) or remove them from (`!silence`) `tips.silence`
/// of the `config.json` at `path`, in one [`update`]. Returns whether the
/// file changed: false when each id was already in (or out of) the list.
/// Fails, writing nothing, on any id that is empty or holds whitespace, and
/// on a `tips` whose shape it cannot keep.
pub(crate) fn set_tips_silenced(
    path: &Path,
    ids: &[String],
    silence: bool,
) -> anyhow::Result<bool> {
    if let Some(bad) = ids.iter().find(|id| !valid_id(id)) {
        anyhow::bail!("{bad:?} is not a tip id; nothing written");
    }
    update(path, |object| {
        // Entries that are no tip id are kept as written: they are the
        // user's to fix, and the reader warns about them.
        let mut list = silence_list(object)
            .map_err(|e| anyhow::anyhow!("{} {e}; not writing it", path.display()))?;
        let before = list.clone();
        for id in ids {
            let listed = list.iter().any(|i| i.as_str() == Some(id));
            if silence && !listed {
                list.push(Value::String(id.clone()));
            } else if !silence && listed {
                list.retain(|i| i.as_str() != Some(id));
            }
        }
        if list == before {
            return Ok(false);
        }
        let tips = object
            .entry("tips")
            .or_insert_with(|| Value::Object(Map::new()));
        if let Value::Object(tips) = tips {
            tips.insert("silence".into(), Value::Array(list));
        }
        Ok(true)
    })
}

/// [`set_tips_silenced`] of one id.
#[cfg(test)]
fn set_tip_silenced(path: &Path, id: &str, silence: bool) -> anyhow::Result<bool> {
    set_tips_silenced(path, &[id.to_owned()], silence)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn read_json(path: &Path) -> Value {
        serde_json::from_str(&fs::read_to_string(path).unwrap()).unwrap()
    }

    #[test]
    fn tips_silence_reads_the_list_and_reports_a_missing_file_or_key_as_empty() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.json");
        assert_eq!(tips_silence(&path), Ok(vec![]), "missing file");
        fs::write(&path, r#"{"editor": "zed"}"#).unwrap();
        assert_eq!(tips_silence(&path), Ok(vec![]), "missing tips");
        fs::write(&path, r#"{"tips": {}}"#).unwrap();
        assert_eq!(tips_silence(&path), Ok(vec![]), "missing silence");
        fs::write(&path, r#"{"tips": {"silence": ["a", "b"]}}"#).unwrap();
        assert_eq!(tips_silence(&path), Ok(vec!["a".into(), "b".into()]));
    }

    #[test]
    fn tips_silence_refuses_an_invalid_value() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.json");
        for (text, wanted) in [
            (r#"{"tips": ["a"]}"#, "\"tips\" must be an object"),
            (r#"{"tips": {"silence": "a"}}"#, "must be a list of tip ids"),
            ("[]", "is not a JSON object"),
            ("{", "failed to parse"),
        ] {
            fs::write(&path, text).unwrap();
            let err = tips_silence(&path).unwrap_err();
            assert!(err.contains(wanted), "{text}: {err}");
            assert!(err.contains(&path.display().to_string()), "{text}: {err}");
        }
    }

    /// One entry that is not a tip id is dropped with a warning naming the
    /// file, as a bad flag or config.axl id is; the list's other ids count.
    #[test]
    fn a_bad_entry_drops_only_itself() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.json");
        fs::write(
            &path,
            r#"{"tips": {"silence": ["a", 1, "", "b c", "x,y", "d"]}}"#,
        )
        .unwrap();
        let (ids, warnings) = read_silence(&path).unwrap();
        assert_eq!(ids, ["a", "d"]);
        assert_eq!(warnings.len(), 4, "{warnings:?}");
        for w in &warnings {
            assert!(w.contains(&path.display().to_string()), "{w}");
            assert!(w.contains("is not a tip id, so it silences nothing"), "{w}");
        }
        assert!(
            warnings[3].contains("\"x,y\"") && warnings[3].contains("split it into list items")
        );
        assert!(!warnings[0].contains("split"), "{}", warnings[0]);
        let found = config_silence(Some(&path), None);
        assert_eq!(found.user, ["a", "d"]);
        assert_eq!(found.warnings.len(), 4);

        // The writer keeps the bad entries as written, for the user to fix.
        assert!(set_tip_silenced(&path, "e", true).unwrap());
        assert!(set_tip_silenced(&path, "a", false).unwrap());
        assert_eq!(
            read_json(&path)["tips"]["silence"],
            serde_json::json!([1, "", "b c", "x,y", "d", "e"])
        );
    }

    #[test]
    fn config_silence_reads_both_files_and_warns_on_an_invalid_one() {
        let dir = tempfile::tempdir().unwrap();
        let user = dir.path().join("user.json");
        let repo = dir.path().join("repo.json");
        fs::write(&user, r#"{"tips": {"silence": ["a", "b"]}}"#).unwrap();
        fs::write(&repo, r#"{"tips": {"silence": ["b", "c"]}}"#).unwrap();
        let found = config_silence(Some(&user), Some(&repo));
        assert_eq!(found.user, ["a", "b"]);
        assert_eq!(found.repo, ["b", "c"]);
        assert!(found.warnings.is_empty());
        fs::write(&repo, r#"{"tips": {"silence": "c"}}"#).unwrap();
        let found = config_silence(Some(&user), Some(&repo));
        assert_eq!(found.user, ["a", "b"], "the valid one kept");
        assert!(found.repo.is_empty());
        assert!(
            found.warnings[0].contains("repo.json"),
            "{:?}",
            found.warnings
        );
        assert!(found.warnings[0].ends_with("; ignoring it"));
        let found = config_silence(None, None);
        assert!(found.user.is_empty() && found.repo.is_empty() && found.warnings.is_empty());
    }

    #[test]
    fn silencing_creates_a_missing_file_and_directory() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("home").join(".aspect").join("config.json");
        assert!(set_tip_silenced(&path, "remote-miss", true).unwrap());
        assert_eq!(
            read_json(&path),
            serde_json::json!({"tips": {"silence": ["remote-miss"]}})
        );
    }

    #[test]
    fn silencing_is_idempotent_and_unsilencing_removes_the_id() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.json");
        assert!(set_tip_silenced(&path, "a", true).unwrap());
        assert!(
            !set_tip_silenced(&path, "a", true).unwrap(),
            "already silenced"
        );
        assert!(set_tip_silenced(&path, "b", true).unwrap());
        assert_eq!(tips_silence(&path), Ok(vec!["a".into(), "b".into()]));
        assert!(set_tip_silenced(&path, "a", false).unwrap());
        assert!(!set_tip_silenced(&path, "a", false).unwrap(), "already out");
        assert_eq!(tips_silence(&path), Ok(vec!["b".into()]));
        let missing = dir.path().join("missing.json");
        assert!(!set_tip_silenced(&missing, "a", false).unwrap());
        assert!(!missing.exists(), "an unsilence of nothing writes nothing");
    }

    #[test]
    fn silencing_keeps_the_other_keys_and_the_mode() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.json");
        fs::write(
            &path,
            r#"{"editor": "zed", "deployments": [{"name": "x"}], "tips": {"later": 1}}"#,
        )
        .unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            fs::set_permissions(&path, fs::Permissions::from_mode(0o600)).unwrap();
        }
        assert!(set_tip_silenced(&path, "a", true).unwrap());
        assert_eq!(
            read_json(&path),
            serde_json::json!({
                "editor": "zed",
                "deployments": [{"name": "x"}],
                "tips": {"later": 1, "silence": ["a"]},
            })
        );
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = fs::metadata(&path).unwrap().permissions().mode() & 0o777;
            assert_eq!(mode, 0o600);
        }
        let leftovers: Vec<_> = fs::read_dir(dir.path())
            .unwrap()
            .map(|e| e.unwrap().file_name())
            .filter(|n| n != "config.json" && n != ".config.json.lock")
            .collect();
        assert!(leftovers.is_empty(), "{leftovers:?}");
    }

    #[test]
    fn silencing_refuses_a_file_or_tips_it_cannot_keep() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.json");
        for text in ["[]", "{", r#"{"tips": {"silence": "a"}}"#, r#"{"tips": 1}"#] {
            fs::write(&path, text).unwrap();
            let err = set_tip_silenced(&path, "a", true).unwrap_err().to_string();
            assert!(err.contains("not writing it"), "{text}: {err}");
            assert_eq!(fs::read_to_string(&path).unwrap(), text, "left as it was");
        }
        assert!(set_tip_silenced(&path, "", true).is_err());
        assert!(set_tip_silenced(&path, "a b", true).is_err());
    }

    #[test]
    fn the_lock_file_is_the_users_alone_and_an_unlockable_one_still_writes() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.json");
        assert!(set_tip_silenced(&path, "a", true).unwrap());
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let lock = dir.path().join(".config.json.lock");
            let mode = fs::metadata(&lock).unwrap().permissions().mode() & 0o777;
            assert_eq!(mode, 0o600);
            // A lock that cannot be opened read-write, as on a home whose
            // filesystem refuses locks: the write goes on without it.
            fs::remove_file(&lock).unwrap();
            fs::create_dir(&lock).unwrap();
            let mut warned = Vec::new();
            let add = |object: &mut Map<String, Value>| {
                object.insert("k".into(), 1.into());
                Ok(true)
            };
            assert!(update_warning(&path, add, |w| warned.push(w.to_owned())).unwrap());
            assert!(
                warned[0].contains("was written without a lock"),
                "{warned:?}"
            );
            assert!(set_tip_silenced(&path, "b", true).unwrap());
            assert_eq!(tips_silence(&path), Ok(vec!["a".into(), "b".into()]));
        }
    }

    /// A read-only `~/.aspect` can neither be locked nor written: one error,
    /// no warning about the lock.
    #[cfg(unix)]
    #[test]
    fn a_read_only_directory_is_one_error() {
        use std::os::unix::fs::PermissionsExt;
        let dir = tempfile::tempdir().unwrap();
        let aspect = dir.path().join(".aspect");
        fs::create_dir(&aspect).unwrap();
        fs::set_permissions(&aspect, fs::Permissions::from_mode(0o555)).unwrap();
        if fs::write(aspect.join("probe"), "").is_ok() {
            // Root writes a read-only directory anyway.
            return;
        }
        let path = aspect.join("config.json");
        let mut warned = Vec::new();
        let add = |object: &mut Map<String, Value>| {
            object.insert("k".into(), 1.into());
            Ok(true)
        };
        let err = update_warning(&path, add, |w| warned.push(w.to_owned()));
        fs::set_permissions(&aspect, fs::Permissions::from_mode(0o755)).unwrap();
        let err = err.unwrap_err().to_string();
        assert!(err.contains("failed to write"), "{err}");
        assert!(warned.is_empty(), "{warned:?}");
    }

    #[cfg(unix)]
    #[test]
    fn another_users_file_is_refused() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.json");
        fs::write(&path, "{}").unwrap();
        let meta = fs::metadata(&path).unwrap();
        let owner = std::os::unix::fs::MetadataExt::uid(&meta);
        assert!(refuse(&path, &meta, Some(owner)).is_ok());
        assert!(refuse(&path, &meta, None).is_ok());
        let err = refuse(&path, &meta, Some(owner + 1))
            .unwrap_err()
            .to_string();
        assert!(err.contains("is owned by another user"), "{err}");
        assert!(err.contains(&path.display().to_string()), "{err}");
    }

    /// The lock stays in `~/.aspect` beside the link, not in the dotfiles
    /// checkout the link points into.
    #[cfg(unix)]
    #[test]
    fn the_lock_is_beside_a_symlink_not_its_target() {
        let dir = tempfile::tempdir().unwrap();
        let aspect = dir.path().join(".aspect");
        let dotfiles = dir.path().join("dotfiles");
        fs::create_dir_all(&aspect).unwrap();
        fs::create_dir_all(&dotfiles).unwrap();
        let link = aspect.join("config.json");
        std::os::unix::fs::symlink(dotfiles.join("aspect.json"), &link).unwrap();
        assert!(set_tip_silenced(&link, "a", true).unwrap());
        assert!(aspect.join(".config.json.lock").exists());
        let in_dotfiles: Vec<_> = fs::read_dir(&dotfiles)
            .unwrap()
            .map(|e| e.unwrap().file_name())
            .collect();
        assert_eq!(in_dotfiles, ["aspect.json"]);
    }

    #[test]
    fn a_comma_is_no_tip_id() {
        assert!(!valid_id("a,b"));
        assert!(!valid_id("a b"));
        assert!(!valid_id(""));
        assert!(valid_id("remote-miss"));
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.json");
        let err = set_tip_silenced(&path, "a,b", true)
            .unwrap_err()
            .to_string();
        assert!(err.contains("\"a,b\" is not a tip id"), "{err}");
    }

    #[test]
    fn an_empty_file_is_an_empty_object_and_a_bom_or_comment_is_refused_saying_why() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.json");
        fs::write(&path, " \n").unwrap();
        assert_eq!(tips_silence(&path), Ok(vec![]));
        assert!(set_tip_silenced(&path, "a", true).unwrap());
        assert_eq!(
            read_json(&path),
            serde_json::json!({"tips": {"silence": ["a"]}})
        );
        for (text, why) in [
            ("\u{feff}{}", "byte order mark"),
            ("{\n  // mine\n}", "holds a comment"),
        ] {
            fs::write(&path, text).unwrap();
            let err = set_tip_silenced(&path, "b", true).unwrap_err().to_string();
            assert!(err.contains(why) && err.contains("not writing it"), "{err}");
            assert_eq!(fs::read_to_string(&path).unwrap(), text, "left as it was");
        }
    }

    #[cfg(unix)]
    #[test]
    fn a_read_only_file_is_refused() {
        use std::os::unix::fs::PermissionsExt;
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.json");
        fs::write(&path, "{}").unwrap();
        fs::set_permissions(&path, fs::Permissions::from_mode(0o444)).unwrap();
        let err = set_tip_silenced(&path, "a", true).unwrap_err().to_string();
        assert!(err.contains("is read-only"), "{err}");
        assert!(err.contains(&path.display().to_string()), "{err}");
        assert_eq!(fs::read_to_string(&path).unwrap(), "{}");
    }

    #[test]
    fn one_bad_id_writes_none_of_them() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.json");
        let ids = ["a".to_owned(), "b c".to_owned(), "d".to_owned()];
        let err = set_tips_silenced(&path, &ids, true)
            .unwrap_err()
            .to_string();
        assert!(err.contains("\"b c\" is not a tip id"), "{err}");
        assert!(!path.exists(), "nothing written");
        let ids = ["a".to_owned(), "d".to_owned(), "a".to_owned()];
        assert!(set_tips_silenced(&path, &ids, true).unwrap());
        assert_eq!(tips_silence(&path), Ok(vec!["a".into(), "d".into()]));
    }

    #[cfg(unix)]
    #[test]
    fn a_dangling_symlink_is_written_through_to_its_target() {
        let dir = tempfile::tempdir().unwrap();
        let link = dir.path().join("config.json");
        std::os::unix::fs::symlink("dotfiles/aspect.json", &link).unwrap();
        assert!(set_tip_silenced(&link, "a", true).unwrap());
        assert!(
            fs::symlink_metadata(&link)
                .unwrap()
                .file_type()
                .is_symlink()
        );
        let target = dir.path().join("dotfiles/aspect.json");
        assert_eq!(tips_silence(&target), Ok(vec!["a".into()]));
    }

    #[test]
    fn silencing_writes_through_a_symlink() {
        let dir = tempfile::tempdir().unwrap();
        let target = dir.path().join("dotfiles.json");
        fs::write(&target, r#"{"editor": "zed"}"#).unwrap();
        let link = dir.path().join("config.json");
        #[cfg(unix)]
        {
            std::os::unix::fs::symlink(&target, &link).unwrap();
            assert!(set_tip_silenced(&link, "a", true).unwrap());
            assert!(
                fs::symlink_metadata(&link)
                    .unwrap()
                    .file_type()
                    .is_symlink()
            );
            assert_eq!(tips_silence(&target), Ok(vec!["a".into()]));
        }
    }

    /// Writers in parallel threads each add their own id; none is lost. The
    /// lock is a `flock`, which is per open file, so threads contend as
    /// processes do.
    #[test]
    fn concurrent_writers_lose_no_id() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join(".aspect").join("config.json");
        let ids: Vec<String> = (0..32).map(|i| format!("tip-{i}")).collect();
        std::thread::scope(|s| {
            for id in &ids {
                let path = &path;
                s.spawn(move || set_tip_silenced(path, id, true).unwrap());
            }
        });
        let mut got = tips_silence(&path).unwrap();
        got.sort();
        let mut wanted = ids.clone();
        wanted.sort();
        assert_eq!(got, wanted);
    }
}
