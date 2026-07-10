//! Persistent credential storage for `aspect auth`: a `{ profile: entry }` map
//! held in one of two backends behind [`CredentialStore`].
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
//! stored value that *is* readable but no longer deserializes into the expected
//! shape (a legacy/corrupt layout) reads as "no credentials" on both backends, so
//! it means "re-run login" rather than a hard error blocking every command.

use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};

use serde::Serialize;
use serde::de::DeserializeOwned;

/// Keyring service name under which all profile entries are stored.
const KEYRING_SERVICE: &str = "Aspect";

/// Environment variable forcing the file backend at a given path (headless CI).
const CREDENTIALS_FILE_ENV: &str = "ASPECT_CREDENTIALS_FILE";

/// The default file-backend path, also the historical credentials location.
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
    /// read. A readable-but-*unparseable* stored value (a legacy/corrupt layout)
    /// is treated as empty on both backends — see `file_load_all` and
    /// `keyring_load_all` — so it means "re-run login", not a hard error on every
    /// command.
    pub(crate) fn load_all<T: DeserializeOwned>(&self) -> anyhow::Result<HashMap<String, T>> {
        match self {
            Self::Keyring => keyring_load_all(),
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

fn keyring_load_all<T: DeserializeOwned>() -> anyhow::Result<HashMap<String, T>> {
    let entry = keyring_entry()?;
    match entry.get_password() {
        // A stored blob that no longer deserializes into the expected shape (a
        // pre-`{profile: entry}` layout, or a legacy bare-string value migrated in
        // from an old `credentials.json`) is treated as "no credentials" so the
        // user simply re-runs `aspect auth login` — matching the file backend
        // (`file_load_all`). A hard error here would instead brick every command
        // until the keyring entry is hand-deleted.
        Ok(raw) => Ok(serde_json::from_str(&raw).unwrap_or_default()),
        Err(keyring::Error::NoEntry) => Ok(HashMap::new()),
        Err(e) => Err(anyhow::anyhow!(
            "failed to read credentials from keyring: {e}"
        )),
    }
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
        // An unparseable file (e.g. a pre-`{profile: entry}` layout) is treated as
        // "no credentials" so the user simply re-runs `aspect auth login`, rather
        // than a hard error that blocks every command until the file is removed.
        Ok(content) => Ok(serde_json::from_str::<HashMap<String, T>>(&content).unwrap_or_default()),
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

    #[test]
    fn file_backend_round_trips_and_is_empty_when_absent() {
        let dir = std::env::temp_dir().join(format!("aspect-cred-test-{}", std::process::id()));
        let path = dir.join("credentials.json");
        let store = CredentialStore::File(path.clone());

        // Absent file → empty, not an error.
        let loaded: HashMap<String, String> = store.load_all().unwrap();
        assert!(loaded.is_empty());

        let mut map = HashMap::new();
        map.insert("default".to_string(), "tok-a".to_string());
        map.insert("acme".to_string(), "tok-b".to_string());
        store.save_all(&map).unwrap();

        let back: HashMap<String, String> = store.load_all().unwrap();
        assert_eq!(back, map);

        // Saving a smaller set replaces the whole file (logout semantics).
        let mut one = HashMap::new();
        one.insert("acme".to_string(), "tok-b".to_string());
        store.save_all(&one).unwrap();
        let back: HashMap<String, String> = store.load_all().unwrap();
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

    #[test]
    fn unparseable_stored_blob_reads_as_empty() {
        // A legacy blob whose profile values are bare strings (the pre-struct
        // layout, or a value migrated in from an old `credentials.json`) no longer
        // deserializes into the entry struct. Both backends must read it as "no
        // credentials" — a re-login prompt — rather than a hard error that blocks
        // every command. This mirrors the tolerance keyring_load_all relies on for
        // the same blob shape (no hermetic keyring to exercise directly).
        let legacy = r#"{"default":"tok-legacy"}"#;
        let parsed: HashMap<String, FakeEntry> =
            serde_json::from_str(legacy).unwrap_or_default();
        assert!(parsed.is_empty(), "legacy bare-string blob → no credentials");

        let dir = std::env::temp_dir().join(format!("aspect-cred-legacy-{}", std::process::id()));
        let path = dir.join("credentials.json");
        fs::create_dir_all(&dir).unwrap();
        fs::write(&path, legacy).unwrap();
        let via_file: HashMap<String, FakeEntry> = file_load_all(&path).unwrap();
        assert!(via_file.is_empty(), "file backend tolerates the same blob");
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
