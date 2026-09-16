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

use std::collections::HashMap;
use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use anyhow::Context;

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

/// The OS keyring is scoped to the OS user, not HOME. In particular, a
/// hermetic wrapper can change HOME/TMPDIR without changing the macOS keychain.
/// Do not use temp_dir(), cache_dir(), or an environment-selected runtime path.
#[cfg(unix)]
fn keyring_lock_path() -> anyhow::Result<PathBuf> {
    // SAFETY: geteuid has no preconditions and always succeeds.
    let uid = unsafe { nix::libc::geteuid() };
    let directory = PathBuf::from(format!("/tmp/aspect-keyring-{uid}"));
    prepare_keyring_lock_directory(&directory, uid)?;
    Ok(directory.join("credentials.lock"))
}

#[cfg(unix)]
fn prepare_keyring_lock_directory(directory: &Path, uid: u32) -> anyhow::Result<()> {
    use std::os::unix::fs::{DirBuilderExt, MetadataExt};

    match fs::DirBuilder::new().mode(0o700).create(directory) {
        Ok(()) => {}
        Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => {}
        Err(e) => return Err(e).context("creating private keyring lock directory"),
    }
    // A predictable path in /tmp must not trust a pre-existing symlink or a
    // directory another user owns/can modify. Never repair or delete it: that
    // could separate callers already holding a lock from later callers.
    let metadata = fs::symlink_metadata(directory)?;
    anyhow::ensure!(
        metadata.is_dir() && metadata.uid() == uid && metadata.mode() & 0o777 == 0o700,
        "keyring lock directory {} must be a private directory owned by OS user {uid}",
        directory.display()
    );
    Ok(())
}

#[cfg(windows)]
fn keyring_lock_path() -> anyhow::Result<PathBuf> {
    // dirs uses the Windows known-folder API here, independently of HOME/TMPDIR.
    Ok(dirs::data_local_dir()
        .context("unable to determine OS user application-data directory")?
        .join("Aspect")
        .join("keyring-credentials.lock"))
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
    pub(crate) fn load_all<T: DeserializeOwned>(&self) -> anyhow::Result<HashMap<String, T>> {
        match self {
            Self::Keyring => keyring_load_all(),
            Self::File(path) => file_load_all(path),
        }
    }

    /// Serialize the entire read/modify/write transaction, including a rotating
    /// refresh grant. All writers share this lock, across threads and processes.
    /// The sidecar is never deleted: replacing it would split the lock domain.
    pub(crate) fn update<T: Serialize + DeserializeOwned, R>(
        &self,
        update: impl FnOnce(&mut HashMap<String, T>) -> anyhow::Result<R>,
    ) -> anyhow::Result<R> {
        let path = match self {
            Self::Keyring => keyring_lock_path()?,
            Self::File(path) => {
                let parent = path
                    .parent()
                    .filter(|p| !p.as_os_str().is_empty())
                    .unwrap_or(Path::new("."));
                fs::create_dir_all(parent)?;
                // Resolve aliases of an existing file before choosing its lock.
                let path = if path.exists() {
                    fs::canonicalize(path)?
                } else {
                    fs::canonicalize(parent)?
                        .join(path.file_name().context("credential file has no name")?)
                };
                let mut name = path.as_os_str().to_os_string();
                name.push(".lock");
                PathBuf::from(name)
            }
        };
        fs::create_dir_all(path.parent().context("credential lock has no parent")?)?;
        let mut options = fs::OpenOptions::new();
        options.read(true).write(true).create(true).truncate(false);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            options.mode(0o600).custom_flags(nix::libc::O_NOFOLLOW);
        }
        let file = options
            .open(&path)
            .context("opening credential store lock")?;
        let mut lock = fd_lock::RwLock::new(file);
        let deadline = Instant::now() + Duration::from_secs(60);
        let _guard = loop {
            match lock.try_write() {
                Ok(guard) => break guard,
                Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                    anyhow::ensure!(
                        Instant::now() < deadline,
                        "timed out waiting for another process to update credentials; retry the command"
                    );
                    std::thread::sleep(Duration::from_millis(20));
                }
                Err(e) => return Err(e).context("locking credential store"),
            }
        };
        let mut map = self.load_all()?;
        let result = update(&mut map)?;
        self.save_all(&map)
            .context("saving credentials after update")?;
        Ok(result)
    }

    /// Only call while holding the update lock (or setting up a test fixture).
    fn save_all<T: Serialize>(&self, map: &HashMap<String, T>) -> anyhow::Result<()> {
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
    if let Some(parent) = path.parent().filter(|p| !p.as_os_str().is_empty()) {
        fs::create_dir_all(parent)
            .map_err(|e| anyhow::anyhow!("failed to create {}: {e}", parent.display()))?;
    }
    let json = serde_json::to_string_pretty(map)
        .map_err(|e| anyhow::anyhow!("failed to serialize credentials: {e}"))?;
    // Follow an existing symlink consistently with the lock path, then replace
    // atomically so unlocked readers see either complete version. NamedTempFile
    // is private (0600 on Unix) from creation, before any credentials are written.
    let destination = if path.exists() {
        fs::canonicalize(path)?
    } else {
        path.to_path_buf()
    };
    let parent = destination
        .parent()
        .filter(|p| !p.as_os_str().is_empty())
        .unwrap_or(Path::new("."));
    let mut temp = tempfile::NamedTempFile::new_in(parent)?;
    temp.write_all(json.as_bytes())?;
    temp.as_file().sync_all()?;
    temp.persist(&destination)
        .map_err(|e| e.error)
        .with_context(|| format!("replacing credentials in {}", destination.display()))?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Invoked in a subprocess so HOME/TMPDIR overrides never mutate the test
    /// runner's environment. A separate lock name avoids blocking real users.
    #[cfg(unix)]
    #[test]
    fn keyring_lock_subprocess_probe() {
        let Some(report) = std::env::var_os("ASPECT_KEYRING_LOCK_TEST_REPORT") else {
            return;
        };
        let name = std::env::var_os("ASPECT_KEYRING_LOCK_TEST_NAME").unwrap();
        let path = keyring_lock_path().unwrap().with_file_name(name);
        let file = fs::OpenOptions::new()
            .read(true)
            .write(true)
            .open(&path)
            .unwrap();
        let mut lock = fd_lock::RwLock::new(file);
        assert_eq!(
            lock.try_write().unwrap_err().kind(),
            std::io::ErrorKind::WouldBlock
        );
        fs::write(report, path.to_str().unwrap()).unwrap();
    }

    #[cfg(unix)]
    #[test]
    fn keyring_lock_is_shared_across_home_and_temp_overrides() {
        use std::os::unix::fs::PermissionsExt;
        use std::process::Command;

        let dir = tempfile::tempdir().unwrap();
        let production_path = keyring_lock_path().unwrap();
        let test_file = tempfile::NamedTempFile::new_in(production_path.parent().unwrap()).unwrap();
        let path = test_file.path();
        let name = path.file_name().unwrap();
        let mut lock = fd_lock::RwLock::new(test_file.reopen().unwrap());
        let _guard = lock.write().unwrap();
        for index in 0..2 {
            // HOME is a regular file, so trying to create HOME/.aspect fails even
            // when this test runs with permissions that bypass read-only bits.
            let home = dir.path().join(format!("unwritable-home-{index}"));
            fs::write(&home, "not a directory").unwrap();
            fs::set_permissions(&home, fs::Permissions::from_mode(0o400)).unwrap();
            let report = dir.path().join(format!("report-{index}"));
            let output = Command::new(std::env::current_exe().unwrap())
                .args([
                    "--exact",
                    "engine::aspect::credential_store::tests::keyring_lock_subprocess_probe",
                    "--nocapture",
                ])
                .env("HOME", home)
                .env("TMPDIR", dir.path().join(format!("different-temp-{index}")))
                .env(
                    "XDG_RUNTIME_DIR",
                    dir.path().join(format!("different-runtime-{index}")),
                )
                .env("ASPECT_KEYRING_LOCK_TEST_REPORT", &report)
                .env("ASPECT_KEYRING_LOCK_TEST_NAME", name)
                .output()
                .unwrap();
            assert!(
                output.status.success(),
                "{}\n{}",
                String::from_utf8_lossy(&output.stdout),
                String::from_utf8_lossy(&output.stderr)
            );
            assert_eq!(fs::read_to_string(report).unwrap(), path.to_str().unwrap());
        }
    }

    #[cfg(unix)]
    #[test]
    fn keyring_lock_directory_rejects_symlinks_and_unsafe_permissions() {
        use std::os::unix::fs::{PermissionsExt, symlink};
        let dir = tempfile::tempdir().unwrap();
        let private = dir.path().join("private");
        // SAFETY: geteuid has no preconditions.
        let uid = unsafe { nix::libc::geteuid() };
        prepare_keyring_lock_directory(&private, uid).unwrap();
        prepare_keyring_lock_directory(&private, uid).unwrap();
        assert!(prepare_keyring_lock_directory(&private, uid.wrapping_add(1)).is_err());
        let alias = dir.path().join("alias");
        symlink(&private, &alias).unwrap();
        assert!(prepare_keyring_lock_directory(&alias, uid).is_err());
        fs::set_permissions(&private, fs::Permissions::from_mode(0o777)).unwrap();
        assert!(prepare_keyring_lock_directory(&private, uid).is_err());
    }

    #[test]
    fn concurrent_updates_preserve_other_profiles_and_deployments() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("credentials.json");
        let start = std::sync::Barrier::new(8);
        std::thread::scope(|scope| {
            for index in 0..8 {
                let path = &path;
                let start = &start;
                scope.spawn(move || {
                    start.wait();
                    CredentialStore::File(path.clone())
                        .update(|map: &mut HashMap<String, HashMap<String, String>>| {
                            // Widen the lost-update window; these are separate opens,
                            // just as independent processes use separate file handles.
                            std::thread::sleep(Duration::from_millis(10));
                            map.entry(format!("profile-{}", index % 2))
                                .or_default()
                                .insert(format!("deployment-{index}"), format!("rotated-{index}"));
                            Ok(())
                        })
                        .unwrap();
                });
            }
        });
        let map: HashMap<String, HashMap<String, String>> =
            CredentialStore::File(path).load_all().unwrap();
        for index in 0..8 {
            assert_eq!(
                map[&format!("profile-{}", index % 2)][&format!("deployment-{index}")],
                format!("rotated-{index}")
            );
        }
    }

    #[test]
    fn failed_update_does_not_replace_credentials_and_releases_lock() {
        let dir = tempfile::tempdir().unwrap();
        let store = CredentialStore::File(dir.path().join("credentials.json"));
        store
            .update(|map: &mut HashMap<String, String>| {
                map.insert("session".into(), "original".into());
                Ok(())
            })
            .unwrap();
        let result = store.update(|map: &mut HashMap<String, String>| -> anyhow::Result<()> {
            map.clear();
            anyhow::bail!("simulated network failure")
        });
        assert!(result.is_err());
        store
            .update(|map: &mut HashMap<String, String>| {
                assert_eq!(map["session"], "original");
                Ok(())
            })
            .unwrap();
    }

    #[cfg(unix)]
    #[test]
    fn updating_a_symlink_preserves_the_link_and_target() {
        let dir = tempfile::tempdir().unwrap();
        let target = dir.path().join("credentials.json");
        let alias = dir.path().join("alias.json");
        let store = CredentialStore::File(target.clone());
        store
            .save_all(&HashMap::from([("session".to_string(), "original")]))
            .unwrap();
        std::os::unix::fs::symlink(&target, &alias).unwrap();
        CredentialStore::File(alias.clone())
            .update(|map: &mut HashMap<String, String>| {
                map.insert("session".into(), "rotated".into());
                Ok(())
            })
            .unwrap();
        assert!(alias.is_symlink());
        assert_eq!(store.load_all::<String>().unwrap()["session"], "rotated");
    }

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
