//! Persistent credential storage for `aspect auth`: a `{ profile: T }` map held in
//! one of two backends behind [`CredentialStore`].
//!
//! Generic in `T`, and deliberately incurious about it — `auth` stores a map of
//! its own (a credential per deployment) as the value, which this never has to
//! know. What it owns is backend selection, whole-map read/write, and the
//! tolerance rules below.
//!
//! - **keyring** - the OS secret service (macOS Keychain, the Linux kernel's
//!   session keyring, Windows Credential Manager), holding the whole map as one
//!   entry. The default on developer machines.
//! - **file** - a `0600` JSON file. The fallback where no secret service is
//!   available (a CI container with no session keyring, a macOS daemon or
//!   fresh `HOME` with no default keychain, or a default keychain whose file
//!   was deleted), or when forced via `$ASPECT_CREDENTIALS_FILE`.
//!
//! CI relies on the file backend: `aspect auth login --with-api-token` persists
//! the exchanged JWT, and a later step of the same job reads it back. A `0600`
//! file on the job's filesystem survives between steps and is torn down with the
//! job; persistence only needs to span steps within one job. (The
//! `$ASPECT_API_TOKEN` env var is a separate in-memory path that never persists.)
//!
//! Backend selection (`CredentialStore::resolve`): `$ASPECT_CREDENTIALS_FILE`
//! forces the file backend at that path; otherwise the keyring is used when its
//! store exists ([`keyring_available`], probed once per process), else the file
//! backend at the default path.
//!
//! Read/write semantics: an absent entry/file reads as "no credentials". A
//! genuine read failure — the secret service unreachable, or a file that can't be
//! read — is surfaced as an error (never an empty map), so a subsequent whole-set
//! `save_all` cannot silently overwrite credentials it only failed to read. A
//! stored value that *is* readable but unparseable (a legacy/corrupt layout) also
//! reads as "no credentials" (see [`parse_stored_map`]), so it means "re-run
//! login" rather than a hard error blocking every command.

use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::OnceLock;

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
    /// backend; otherwise prefer the keyring when it has a store
    /// ([`keyring_available`]), else fall back to the default file path.
    pub(crate) fn resolve() -> anyhow::Result<Self> {
        Self::resolve_with(std::env::var_os(CREDENTIALS_FILE_ENV))
    }

    /// [`resolve`](Self::resolve) with the `$ASPECT_CREDENTIALS_FILE` value
    /// passed in.
    fn resolve_with(file_override: Option<std::ffi::OsString>) -> anyhow::Result<Self> {
        if let Some(path) = file_override {
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

/// Whether the OS secret service can hold credentials, probed once per process.
///
/// Opening the entry fails where no service exists at all (Linux opens the
/// session keyring up front, so a container without one fails here). Other
/// platforms open lazily, so a missing store only shows on first access:
/// [`store_present`] checks for one and [`keyring_usable`] judges the result.
fn keyring_available() -> bool {
    static AVAILABLE: OnceLock<bool> = OnceLock::new();
    *AVAILABLE.get_or_init(|| keyring_entry().is_ok_and(|entry| store_present(&entry)))
}

/// Whether the user's default keychain exists. Looks the keychain up the way
/// keyring does before every access, then asks for its status, without
/// reading the entry, so the probe never raises a keychain access or unlock
/// prompt. The status catches a default that names a keychain file since
/// deleted (`errSecNoSuchKeychain`), which the lookup alone does not: saving
/// to it fails, and keyring reports it as `NoStorageAccess`.
#[cfg(target_os = "macos")]
fn store_present(_: &keyring::Entry) -> bool {
    use security_framework::os::macos::keychain::{SecKeychain, SecPreferencesDomain};
    keyring_usable(
        &SecKeychain::default_for_domain(SecPreferencesDomain::User)
            .and_then(|keychain| keychain_status(&keychain))
            .map_err(keyring::macos::decode_error),
    )
}

/// `SecKeychainGetStatus`, which security-framework does not wrap. Fails for a
/// keychain whose file is gone; a locked keychain is a status, not a failure.
#[cfg(target_os = "macos")]
fn keychain_status(
    keychain: &security_framework::os::macos::keychain::SecKeychain,
) -> security_framework::base::Result<()> {
    use core_foundation::base::TCFType;
    unsafe extern "C" {
        fn SecKeychainGetStatus(keychain: *const std::ffi::c_void, status: *mut u32) -> i32;
    }
    let mut status = 0;
    // SAFETY: a live keychain reference and a valid out-pointer.
    match unsafe { SecKeychainGetStatus(keychain.as_CFTypeRef(), &mut status) } {
        0 => Ok(()),
        code => Err(security_framework::base::Error::from_code(code)),
    }
}

/// Whether a read of the entry finds a store (Windows reports a missing logon
/// session only on access).
#[cfg(not(target_os = "macos"))]
fn store_present(entry: &keyring::Entry) -> bool {
    keyring_usable(&entry.get_password())
}

/// Whether a keyring access leaves the keyring usable. Only a missing store rules
/// it out (`NoStorageAccess`, or macOS's "no default keychain", which keyring
/// reports as a `PlatformFailure`): there is nothing stored there to lose. Any
/// other failure keeps the keyring, so [`CredentialStore::load_all`] reports it
/// instead of reading an empty file over credentials it holds.
fn keyring_usable<T>(access: &keyring::Result<T>) -> bool {
    match access {
        Err(keyring::Error::NoStorageAccess(_)) => false,
        Err(keyring::Error::PlatformFailure(e)) => !no_default_keychain(e.as_ref()),
        _ => true,
    }
}

/// Whether `err` is macOS's `errSecNoDefaultKeychain`: the user has no default
/// keychain, as under a daemon or a fresh `HOME`.
#[cfg(target_os = "macos")]
fn no_default_keychain(err: &(dyn std::error::Error + Send + Sync + 'static)) -> bool {
    const ERR_SEC_NO_DEFAULT_KEYCHAIN: i32 = -25307;
    err.downcast_ref::<security_framework::base::Error>()
        .is_some_and(|e| e.code() == ERR_SEC_NO_DEFAULT_KEYCHAIN)
}

#[cfg(not(target_os = "macos"))]
fn no_default_keychain(_: &(dyn std::error::Error + Send + Sync + 'static)) -> bool {
    false
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
    fn only_a_missing_store_rules_the_keyring_out() {
        let platform = || Box::new(std::io::Error::other("platform")) as _;
        assert!(keyring_usable(&Ok("{}")));
        assert!(keyring_usable::<()>(&Err(keyring::Error::NoEntry)));
        assert!(keyring_usable::<()>(&Err(keyring::Error::PlatformFailure(
            platform()
        ))));
        assert!(!keyring_usable::<()>(&Err(
            keyring::Error::NoStorageAccess(platform())
        )));
    }

    /// Goes through keyring's own `decode_error`, which also pins our
    /// `security-framework` to keyring's: a mismatch fails to compile rather than
    /// silently missing the downcast.
    #[cfg(target_os = "macos")]
    #[test]
    fn a_missing_default_keychain_rules_the_keyring_out() {
        let status = |code| {
            Err::<(), _>(keyring::macos::decode_error(
                security_framework::base::Error::from_code(code),
            ))
        };
        assert!(!keyring_usable(&status(-25307)), "errSecNoDefaultKeychain");
        assert!(!keyring_usable(&status(-25294)), "errSecNoSuchKeychain");
        assert!(
            keyring_usable(&status(-25308)),
            "errSecInteractionNotAllowed: a locked keychain"
        );
        assert!(keyring_usable(&status(-25300)), "errSecItemNotFound");
    }

    /// `SecKeychainOpen` does not check the path, so this is a reference to a
    /// keychain file that is gone, as a stale default is.
    #[cfg(target_os = "macos")]
    #[test]
    fn a_missing_keychain_file_fails_its_status() {
        use security_framework::os::macos::keychain::SecKeychain;
        let dir = std::env::temp_dir().join(format!("aspect-cred-kc-{}", std::process::id()));
        let keychain = SecKeychain::open(dir.join("gone.keychain-db")).unwrap();
        let status = keychain_status(&keychain);
        assert_eq!(status.unwrap_err().code(), -25294, "errSecNoSuchKeychain");
        assert!(!keyring_usable(
            &status.map_err(keyring::macos::decode_error)
        ));
    }

    #[test]
    fn env_override_forces_file_backend() {
        let store = CredentialStore::resolve_with(Some("/tmp/aspect-forced.json".into())).unwrap();
        assert!(
            matches!(store, CredentialStore::File(p) if p == PathBuf::from("/tmp/aspect-forced.json"))
        );
    }
}
