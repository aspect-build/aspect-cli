//! Persistent credential storage for `aspect auth`: a `{ profile: T }` map held in
//! one of two backends behind [`CredentialStore`].
//!
//! Generic in `T`, and deliberately incurious about it — `auth` stores a map of
//! its own (a credential per deployment) as the value, which this never has to
//! know. What it owns is backend selection, whole-map read/write, and the
//! tolerance rules below.
//!
//! - **keyring** — the OS secret service (macOS Keychain, Linux Secret Service,
//!   Windows Credential Manager), holding the whole map as one entry. The default
//!   on developer machines.
//! - **file** — a `0600` JSON file. The fallback where no secret service is
//!   available (headless CI has no D-Bus keyring), or when forced via
//!   `$ASPECT_CREDENTIALS_FILE`.
//!
//! CI relies on the file backend: `aspect auth login --with-api-token` persists
//! the exchanged JWT, and a later step of the same job reads it back. A `0600`
//! file on the job's filesystem survives between steps and is torn down with the
//! job; persistence only needs to span steps within one job. (The
//! `$ASPECT_API_TOKEN` env var is a separate in-memory path that never persists.)
//!
//! Backend selection (`CredentialStore::resolve`): `$ASPECT_CREDENTIALS_FILE`
//! forces the file backend at that path; otherwise the keyring is used when its
//! secret service is reachable, else the file backend at the default path.
//!
//! Read/write semantics: an absent entry/file reads as "no credentials". A
//! genuine read failure — the secret service unreachable, or a file that can't be
//! read — is surfaced as an error (never an empty map), so a subsequent whole-set
//! `save_all` cannot silently overwrite credentials it only failed to read. A
//! stored value that *is* readable but unparseable (a legacy/corrupt layout) also
//! reads as "no credentials" (see [`parse_stored_map`]), so it means "re-run
//! login" rather than a hard error blocking every command.
//!
//! An empty keyring falls back to the default-path file, where releases before
//! the keyring backend stored credentials in an older layout that only the
//! caller can read; see [`migrate_legacy_file`].

use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};

use serde::Serialize;
use serde::de::DeserializeOwned;

/// Keyring service name under which all profile entries are stored.
const KEYRING_SERVICE: &str = "Aspect";

/// Environment variable forcing the file backend at a given path (headless CI).
const CREDENTIALS_FILE_ENV: &str = "ASPECT_CREDENTIALS_FILE";

/// The default file-backend path, also where releases before the keyring backend
/// stored credentials.
fn default_file_path() -> anyhow::Result<PathBuf> {
    let home =
        dirs::home_dir().ok_or_else(|| anyhow::anyhow!("unable to determine home directory"))?;
    Ok(home.join(".aspect").join("credentials.json"))
}

/// Where credentials for a profile are persisted. One value type (`T`) is stored
/// per profile name; `T` is (de)serialized as JSON.
pub(crate) enum CredentialStore {
    /// OS secret service; the whole `{ profile: T }` map is one entry under
    /// [`KEYRING_SERVICE`] / [`KEYRING_ACCOUNT`].
    Keyring,
    /// A single `0600` JSON file holding `{ profile: T }`.
    File(PathBuf),
}

impl CredentialStore {
    /// Resolve the active backend: `$ASPECT_CREDENTIALS_FILE` forces the file
    /// backend; otherwise prefer the keyring when its service is reachable, else
    /// fall back to the default file path.
    pub(crate) fn resolve() -> anyhow::Result<Self> {
        if let Some(path) = std::env::var_os(CREDENTIALS_FILE_ENV) {
            return Ok(Self::File(PathBuf::from(path)));
        }
        if keyring_available() {
            Ok(Self::Keyring)
        } else {
            Ok(Self::File(default_file_path()?))
        }
    }

    /// All stored profile → value pairs. An absent entry/file is an empty map
    /// (not logged in). An IO/service read failure (secret service unreachable, a
    /// file that can't be read) is an error rather than an empty map, so a
    /// subsequent `save_all` never overwrites good credentials it merely failed to
    /// read. A readable-but-unparseable stored value (a legacy/corrupt layout) is
    /// treated as empty on both backends — see [`parse_stored_map`] — so it means
    /// "re-run login", not a hard error on every command.
    ///
    /// An empty keyring reads the default-path file through `parse_legacy`, moving
    /// what it recognizes into the keyring; see [`migrate_legacy_file`].
    pub(crate) fn load_all<T, P>(&self, parse_legacy: P) -> anyhow::Result<HashMap<String, T>>
    where
        T: DeserializeOwned + Serialize,
        P: FnOnce(&str) -> Option<HashMap<String, T>>,
    {
        match self {
            Self::Keyring => {
                let map = keyring_load_all()?;
                if !map.is_empty() {
                    return Ok(map);
                }
                match default_file_path() {
                    Ok(path) => Ok(migrate_legacy_file(
                        &path,
                        parse_legacy,
                        keyring_save_all,
                        keyring_load_all,
                    )),
                    Err(_) => Ok(map),
                }
            }
            Self::File(path) => file_load_all(path),
        }
    }

    /// Replace the entire stored set with `map`.
    pub(crate) fn save_all<T: Serialize>(&self, map: &HashMap<String, T>) -> anyhow::Result<()> {
        match self {
            Self::Keyring => keyring_save_all(map),
            Self::File(path) => file_save_all(path, map),
        }
    }
}

/// Keyring account under which the whole credential set is stored as one JSON
/// blob. Storing the full `{ profile: T }` map in a single entry makes writes
/// atomic (one `set_password`), removes the need for a separate profile index,
/// and lets a read cleanly distinguish "no credentials yet" (entry absent) from
/// "service unavailable" (any other error) — so a transient read failure never
/// causes a save to overwrite good credentials with a partial set.
const KEYRING_ACCOUNT: &str = "credentials";

fn keyring_entry() -> anyhow::Result<keyring::Entry> {
    keyring::Entry::new(KEYRING_SERVICE, KEYRING_ACCOUNT)
        .map_err(|e| anyhow::anyhow!("failed to open keyring entry: {e}"))
}

/// Whether the OS secret service is reachable — probed by opening (not reading)
/// the entry. `keyring::Entry::new` fails when no backend is compiled in or the
/// platform service is missing, which is our signal to use the file backend.
fn keyring_available() -> bool {
    keyring_entry().is_ok()
}

/// Parse a stored `{ profile: T }` blob, treating an unparseable one (a corrupt or
/// differently-shaped layout) as "no credentials" rather than an error, so the user
/// re-runs `aspect auth login` instead of every command hard-failing. Shared by
/// both backends so the tolerance is identical.
fn parse_stored_map<T: DeserializeOwned>(raw: &str) -> HashMap<String, T> {
    serde_json::from_str(raw).unwrap_or_default()
}

fn keyring_load_all<T: DeserializeOwned>() -> anyhow::Result<HashMap<String, T>> {
    let entry = keyring_entry()?;
    match entry.get_password() {
        Ok(raw) => Ok(parse_stored_map(&raw)),
        Err(keyring::Error::NoEntry) => Ok(HashMap::new()),
        Err(e) => Err(anyhow::anyhow!(
            "failed to read credentials from keyring: {e}"
        )),
    }
}

/// The credentials in the legacy file at `path`, moved into the keyring through
/// `save` so upgrading users stay logged in.
///
/// `parse` decides what is legacy: it returns `None` for anything it does not
/// recognize as an older layout, the current layout included. A current-layout
/// file at the default path is the file backend's own store, written on a run
/// where the keyring was unreachable, and must stay put so such a run still finds
/// it.
///
/// The credentials are returned only once `load` reads every profile back, and
/// the file is deleted then too. A failed or partial write returns nothing and
/// keeps the file for the next run: handing the credentials out anyway would let
/// a later keyring write adopt them while the file survives, so a logout would
/// bring them back. An unreadable file reads as nothing to migrate rather than
/// failing every command.
fn migrate_legacy_file<T, P, S, L>(path: &Path, parse: P, save: S, load: L) -> HashMap<String, T>
where
    P: FnOnce(&str) -> Option<HashMap<String, T>>,
    S: FnOnce(&HashMap<String, T>) -> anyhow::Result<()>,
    L: FnOnce() -> anyhow::Result<HashMap<String, T>>,
{
    let raw = match fs::read_to_string(path) {
        Ok(raw) => raw,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return HashMap::new(),
        Err(e) => {
            tracing::warn!(
                "not migrating legacy credentials from {}: {e}",
                path.display()
            );
            return HashMap::new();
        }
    };
    let Some(legacy) = parse(&raw).filter(|m| !m.is_empty()) else {
        return HashMap::new();
    };
    let complete = save(&legacy)
        .and_then(|()| load())
        .map(|stored| legacy.keys().all(|k| stored.contains_key(k)));
    match complete {
        Ok(true) => {}
        Ok(false) => {
            tracing::warn!("legacy credentials did not read back from the keyring");
            return HashMap::new();
        }
        Err(e) => {
            tracing::warn!("could not migrate legacy credentials into the keyring: {e}");
            return HashMap::new();
        }
    }
    match fs::remove_file(path) {
        Ok(()) => {}
        // A concurrent invocation migrated the same file first.
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
        Err(e) => tracing::warn!(
            "migrated credentials but could not remove {}: {e}",
            path.display()
        ),
    }
    legacy
}

fn keyring_save_all<T: Serialize>(map: &HashMap<String, T>) -> anyhow::Result<()> {
    let entry = keyring_entry()?;
    if map.is_empty() {
        // `logout --all`: remove the entry entirely (a NoEntry on next load is
        // simply "no credentials").
        return match entry.delete_credential() {
            Ok(()) | Err(keyring::Error::NoEntry) => Ok(()),
            Err(e) => Err(anyhow::anyhow!(
                "failed to clear credentials from keyring: {e}"
            )),
        };
    }
    let json = serde_json::to_string(map)
        .map_err(|e| anyhow::anyhow!("failed to serialize credentials: {e}"))?;
    entry
        .set_password(&json)
        .map_err(|e| anyhow::anyhow!("failed to write credentials to keyring: {e}"))
}

fn file_load_all<T: DeserializeOwned>(path: &Path) -> anyhow::Result<HashMap<String, T>> {
    match fs::read_to_string(path) {
        Ok(content) => Ok(parse_stored_map(&content)),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(HashMap::new()),
        Err(e) => Err(anyhow::anyhow!("failed to read {}: {e}", path.display())),
    }
}

fn file_save_all<T: Serialize>(path: &Path, map: &HashMap<String, T>) -> anyhow::Result<()> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)
            .map_err(|e| anyhow::anyhow!("failed to create {}: {e}", parent.display()))?;
    }
    let json = serde_json::to_string_pretty(map)
        .map_err(|e| anyhow::anyhow!("failed to serialize credentials: {e}"))?;
    fs::write(path, &json)
        .map_err(|e| anyhow::anyhow!("failed to write {}: {e}", path.display()))?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(path, fs::Permissions::from_mode(0o600))
            .map_err(|e| anyhow::anyhow!("failed to set credentials permissions: {e}"))?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A legacy `credentials.json` in a fresh temp dir holding one profile, in
    /// a layout [`legacy`] recognizes.
    fn legacy_file() -> (tempfile::TempDir, PathBuf, HashMap<String, String>) {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("credentials.json");
        fs::write(&path, r#"{"default": 7}"#).unwrap();
        let migrated = HashMap::from([("default".to_string(), "7".to_string())]);
        (dir, path, migrated)
    }

    /// A stand-in legacy parser: integer values are the older layout, string
    /// values (what this store would write) are current and yield `None`.
    fn legacy(raw: &str) -> Option<HashMap<String, String>> {
        let old: HashMap<String, i64> = serde_json::from_str(raw).ok()?;
        Some(old.into_iter().map(|(k, v)| (k, v.to_string())).collect())
    }

    #[test]
    fn migration_moves_the_legacy_file_into_the_keyring() {
        let (_dir, path, migrated) = legacy_file();
        let written = std::cell::RefCell::new(HashMap::new());
        let got = migrate_legacy_file(
            &path,
            legacy,
            |m: &HashMap<String, String>| {
                *written.borrow_mut() = m.clone();
                Ok(())
            },
            || Ok(written.borrow().clone()),
        );
        assert_eq!(got, migrated);
        assert_eq!(*written.borrow(), migrated);
        assert!(
            !path.exists(),
            "a completed migration removes the legacy file"
        );
    }

    #[test]
    fn a_failed_keyring_write_keeps_the_file_and_returns_nothing() {
        let (_dir, path, _) = legacy_file();
        let got = migrate_legacy_file(
            &path,
            legacy,
            |_: &HashMap<String, String>| Err(anyhow::anyhow!("keyring locked")),
            || unreachable!("nothing to read back after a failed write"),
        );
        assert!(got.is_empty());
        assert!(path.exists());
    }

    #[test]
    fn a_partial_read_back_keeps_the_file_and_returns_nothing() {
        let (_dir, path, _) = legacy_file();
        let got = migrate_legacy_file(
            &path,
            legacy,
            |_: &HashMap<String, String>| Ok(()),
            || Ok(HashMap::new()),
        );
        assert!(got.is_empty());
        assert!(path.exists());
    }

    #[test]
    fn a_current_layout_file_is_left_for_the_file_backend() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("credentials.json");
        let current = HashMap::from([("default".to_string(), "tok".to_string())]);
        file_save_all(&path, &current).unwrap();
        let got = migrate_legacy_file(
            &path,
            legacy,
            |_: &HashMap<String, String>| unreachable!("nothing to write"),
            || unreachable!("nothing to read back"),
        );
        assert!(got.is_empty());
        assert_eq!(file_load_all::<String>(&path).unwrap(), current);
    }

    #[test]
    fn a_file_removed_by_a_concurrent_migration_still_migrates() {
        let (_dir, path, migrated) = legacy_file();
        let got = migrate_legacy_file(
            &path,
            legacy,
            |_: &HashMap<String, String>| {
                fs::remove_file(&path).unwrap();
                Ok(())
            },
            || Ok(migrated.clone()),
        );
        assert_eq!(got, migrated);
    }

    #[test]
    fn no_legacy_file_means_nothing_to_migrate() {
        let dir = tempfile::tempdir().unwrap();
        let got: HashMap<String, String> = migrate_legacy_file(
            &dir.path().join("credentials.json"),
            legacy,
            |_| unreachable!("nothing to write"),
            || unreachable!("nothing to read back"),
        );
        assert!(got.is_empty());
    }

    #[test]
    fn file_backend_round_trips_and_is_empty_when_absent() {
        let dir = std::env::temp_dir().join(format!("aspect-cred-test-{}", std::process::id()));
        let path = dir.join("credentials.json");
        let store = CredentialStore::File(path.clone());

        // Absent file → empty, not an error.
        let loaded: HashMap<String, String> = store.load_all(legacy).unwrap();
        assert!(loaded.is_empty());

        let mut map = HashMap::new();
        map.insert("default".to_string(), "tok-a".to_string());
        map.insert("acme".to_string(), "tok-b".to_string());
        store.save_all(&map).unwrap();

        let back: HashMap<String, String> = store.load_all(legacy).unwrap();
        assert_eq!(back, map);

        // Saving a smaller set replaces the whole file (logout semantics).
        let mut one = HashMap::new();
        one.insert("acme".to_string(), "tok-b".to_string());
        store.save_all(&one).unwrap();
        let back: HashMap<String, String> = store.load_all(legacy).unwrap();
        assert_eq!(back, one);

        let _ = fs::remove_dir_all(&dir);
    }

    #[cfg(unix)]
    #[test]
    fn file_backend_is_0600() {
        use std::os::unix::fs::PermissionsExt;
        let dir = std::env::temp_dir().join(format!("aspect-cred-perm-{}", std::process::id()));
        let path = dir.join("credentials.json");
        let store = CredentialStore::File(path.clone());
        let mut map = HashMap::new();
        map.insert("default".to_string(), "tok".to_string());
        store.save_all(&map).unwrap();
        let mode = fs::metadata(&path).unwrap().permissions().mode();
        assert_eq!(mode & 0o777, 0o600);
        let _ = fs::remove_dir_all(&dir);
    }

    #[derive(serde::Deserialize)]
    struct FakeEntry {
        #[allow(dead_code)]
        access_token: String,
    }

    // A legacy blob whose profile values are bare strings (the pre-struct layout)
    // no longer deserializes into the entry struct.
    const LEGACY_BLOB: &str = r#"{"default":"tok-legacy"}"#;

    #[test]
    fn parse_stored_map_tolerates_unparseable_blob() {
        // The shared tolerance both backends rely on: an unparseable blob reads as
        // "no credentials" (a re-login prompt) rather than a hard error blocking
        // every command. This is the keyring crash path, which has no hermetic
        // keyring to drive directly.
        let parsed: HashMap<String, FakeEntry> = parse_stored_map(LEGACY_BLOB);
        assert!(parsed.is_empty());
        assert_eq!(
            parse_stored_map::<String>(r#"not json at all"#).len(),
            0,
            "non-JSON also reads as empty"
        );
    }

    #[test]
    fn file_backend_tolerates_unparseable_blob() {
        let dir = std::env::temp_dir().join(format!("aspect-cred-legacy-{}", std::process::id()));
        let path = dir.join("credentials.json");
        fs::create_dir_all(&dir).unwrap();
        fs::write(&path, LEGACY_BLOB).unwrap();
        let loaded: HashMap<String, FakeEntry> = file_load_all(&path).unwrap();
        assert!(loaded.is_empty());
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn env_override_forces_file_backend() {
        // SAFETY: single-threaded test; var removed before returning.
        unsafe { std::env::set_var(CREDENTIALS_FILE_ENV, "/tmp/aspect-forced.json") };
        let store = CredentialStore::resolve().unwrap();
        assert!(
            matches!(store, CredentialStore::File(p) if p == PathBuf::from("/tmp/aspect-forced.json"))
        );
        unsafe { std::env::remove_var(CREDENTIALS_FILE_ENV) };
    }
}
