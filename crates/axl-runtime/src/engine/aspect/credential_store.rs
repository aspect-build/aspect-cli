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

/// The OS user's home directory as the password database records it, not
/// `$HOME`: the keyring is scoped to the OS user, and a hermetic wrapper can
/// point HOME (or TMPDIR) anywhere without changing the keychain it reaches.
#[cfg(unix)]
fn os_user_home() -> anyhow::Result<PathBuf> {
    use nix::libc;
    use std::ffi::{CStr, OsStr};
    use std::os::unix::ffi::OsStrExt;

    // SAFETY: geteuid has no preconditions and always succeeds.
    let uid = unsafe { libc::geteuid() };
    let mut buf = vec![0 as libc::c_char; 4096];
    loop {
        // SAFETY: zeroed is a valid `passwd` (null pointers, zero ids).
        let mut pwd: libc::passwd = unsafe { std::mem::zeroed() };
        let mut found: *mut libc::passwd = std::ptr::null_mut();
        // SAFETY: every pointer is valid for the call and `buf.len()` is its size.
        let rc =
            unsafe { libc::getpwuid_r(uid, &mut pwd, buf.as_mut_ptr(), buf.len(), &mut found) };
        if rc == libc::ERANGE && buf.len() < 1 << 20 {
            buf.resize(buf.len() * 2, 0);
            continue;
        }
        anyhow::ensure!(
            rc == 0 && !found.is_null() && !pwd.pw_dir.is_null(),
            "unable to determine the home directory of OS user {uid}"
        );
        // SAFETY: getpwuid_r succeeded, so pw_dir is a NUL-terminated string in `buf`.
        let dir = unsafe { CStr::from_ptr(pwd.pw_dir) }.to_bytes();
        anyhow::ensure!(!dir.is_empty(), "OS user {uid} has no home directory");
        return Ok(PathBuf::from(OsStr::from_bytes(dir)));
    }
}

/// A per-OS-user directory for credential locks. Never shared `/tmp`, where
/// another local user could pre-create the path, where periodic cleanup can
/// delete a lock file out from under a holder (splitting the lock domain), and
/// which a private-`/tmp` namespace hides from processes sharing one keyring.
#[cfg(unix)]
fn os_user_lock_dir() -> anyhow::Result<PathBuf> {
    Ok(os_user_home()?.join(".aspect"))
}

#[cfg(windows)]
fn os_user_lock_dir() -> anyhow::Result<PathBuf> {
    // dirs uses the Windows known-folder API here, independently of HOME/TMPDIR.
    Ok(dirs::data_local_dir()
        .context("unable to determine OS user application-data directory")?
        .join("Aspect"))
}

/// The lock serializing keyring updates. Every process of the OS user shares
/// one keychain entry, so every one of them must find this same file.
fn keyring_lock_path() -> anyhow::Result<PathBuf> {
    Ok(os_user_lock_dir()?.join("keyring-credentials.lock"))
}

/// The lock for a credentials file whose own directory the process can't write
/// (an `ASPECT_CREDENTIALS_FILE` in a read-only directory), keyed by the file's
/// canonical path so every process updating that file finds the same one.
fn file_fallback_lock_path(canonical: &Path) -> anyhow::Result<PathBuf> {
    use sha2::Digest;
    let digest = sha2::Sha256::digest(canonical.as_os_str().as_encoded_bytes());
    let key: String = digest[..16].iter().map(|b| format!("{b:02x}")).collect();
    Ok(os_user_lock_dir()?
        .join("locks")
        .join(format!("credentials-{key}.lock")))
}

/// Where credentials at `path` are written, and what their lock is named after:
/// an existing file (or a symlink to one) resolved to its target, else the
/// would-be file in its canonicalized directory, which is created if missing.
/// The lock and the save both go through here, so they always agree.
fn canonical_credentials_path(path: &Path) -> anyhow::Result<PathBuf> {
    if path.exists() {
        return fs::canonicalize(path)
            .with_context(|| format!("failed to resolve {}", path.display()));
    }
    let parent = path
        .parent()
        .filter(|p| !p.as_os_str().is_empty())
        .unwrap_or(Path::new("."));
    fs::create_dir_all(parent).with_context(|| format!("failed to create {}", parent.display()))?;
    Ok(fs::canonicalize(parent)
        .with_context(|| format!("failed to resolve {}", parent.display()))?
        .join(path.file_name().context("credential file has no name")?))
}

/// Open (creating) a lock file, private and never through a symlink.
fn open_lock_file(path: &Path) -> std::io::Result<fs::File> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    let mut options = fs::OpenOptions::new();
    options.read(true).write(true).create(true).truncate(false);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600).custom_flags(nix::libc::O_NOFOLLOW);
    }
    options.open(path)
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
    ///
    /// The store is written back only when `update` changed the map. A no-op
    /// (a logout of something not stored, a refresh another process already
    /// did) then never rewrites the store, so it can't replace a stored value
    /// this binary couldn't parse (read as empty, see [`parse_stored_map`]) with
    /// an empty one, and doesn't create a credentials file that wasn't there.
    pub(crate) fn update<T: Serialize + DeserializeOwned, R>(
        &self,
        update: impl FnOnce(&mut HashMap<String, T>) -> anyhow::Result<R>,
    ) -> anyhow::Result<R> {
        self.locked(|| {
            let mut map = self.load_all()?;
            let before = serde_json::to_value(&map)?;
            let result = update(&mut map)?;
            if serde_json::to_value(&map)? != before {
                self.save_all(&map)
                    .context("saving credentials after update")?;
            }
            Ok(result)
        })
    }

    /// Remove every stored credential (`logout --all`) without reading the store
    /// first, so a store that can't be read or parsed can still be cleared.
    pub(crate) fn clear(&self) -> anyhow::Result<()> {
        self.locked(|| self.save_all(&HashMap::<String, ()>::new()))
    }

    /// The lock file for this store. A file store's lock sits next to it (so it
    /// is shared by whoever shares the file's directory), unless that directory
    /// can't be written, in which case it is a per-user one keyed by the file.
    /// Lock files are never deleted: replacing one would split the lock domain.
    fn lock_file(&self) -> anyhow::Result<fs::File> {
        let path = match self {
            Self::Keyring => keyring_lock_path()?,
            Self::File(path) => {
                let canonical = canonical_credentials_path(path)?;
                let mut sidecar = canonical.as_os_str().to_os_string();
                sidecar.push(".lock");
                match open_lock_file(Path::new(&sidecar)) {
                    Ok(file) => return Ok(file),
                    Err(e)
                        if matches!(
                            e.kind(),
                            std::io::ErrorKind::PermissionDenied
                                | std::io::ErrorKind::ReadOnlyFilesystem
                        ) =>
                    {
                        file_fallback_lock_path(&canonical)?
                    }
                    Err(e) => {
                        return Err(e).with_context(|| {
                            format!("opening credential store lock {}", sidecar.display())
                        });
                    }
                }
            }
        };
        open_lock_file(&path)
            .with_context(|| format!("opening credential store lock {}", path.display()))
    }

    /// Run `f` holding this store's exclusive lock.
    fn locked<R>(&self, f: impl FnOnce() -> anyhow::Result<R>) -> anyhow::Result<R> {
        let mut lock = fd_lock::RwLock::new(self.lock_file()?);
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
        f()
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
    let json = serde_json::to_string_pretty(map)
        .map_err(|e| anyhow::anyhow!("failed to serialize credentials: {e}"))?;
    // The same resolved path the lock is named after; then replace atomically so
    // unlocked readers see either complete version. NamedTempFile is private
    // (0600 on Unix) from creation, before any credentials are written.
    let destination = canonical_credentials_path(path)?;
    let parent = destination.parent().unwrap_or(Path::new("."));
    let mut temp = match tempfile::NamedTempFile::new_in(parent) {
        Ok(temp) => temp,
        // A writable file in a directory this process can't write.
        Err(e) if e.kind() == std::io::ErrorKind::PermissionDenied => {
            return write_in_place(&destination, &json);
        }
        Err(e) => {
            return Err(e)
                .with_context(|| format!("creating a temporary file in {}", parent.display()));
        }
    };
    temp.write_all(json.as_bytes())?;
    temp.as_file().sync_all()?;
    match temp.persist(&destination) {
        Ok(_) => Ok(()),
        // A file bind-mounted on its own (devcontainers, `docker -v file:file`)
        // can't be renamed over: EBUSY, or EXDEV on some kernels.
        Err(e) if cannot_rename_over(&e.error) => write_in_place(&destination, &json),
        Err(e) => Err(e.error)
            .with_context(|| format!("replacing credentials in {}", destination.display())),
    }
}

fn cannot_rename_over(e: &std::io::Error) -> bool {
    #[cfg(unix)]
    {
        matches!(
            e.raw_os_error(),
            Some(nix::libc::EBUSY) | Some(nix::libc::EXDEV)
        )
    }
    #[cfg(not(unix))]
    {
        let _ = e;
        false
    }
}

/// Overwrite `destination` in place, for where an atomic replace can't work. Still
/// under the store lock, so writers stay serialized; only an unlocked reader can
/// catch it half-written, which reads as "no credentials" for that one read.
fn write_in_place(destination: &Path, json: &str) -> anyhow::Result<()> {
    let mut options = fs::OpenOptions::new();
    options.write(true).create(true).truncate(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    let mut file = options
        .open(destination)
        .with_context(|| format!("writing credentials to {}", destination.display()))?;
    file.write_all(json.as_bytes())?;
    file.sync_all()?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Invoked in a subprocess so HOME/TMPDIR overrides never mutate the test
    /// runner's environment. Only reports the path; nothing is created.
    #[cfg(unix)]
    #[test]
    fn keyring_lock_subprocess_probe() {
        let Some(report) = std::env::var_os("ASPECT_KEYRING_LOCK_TEST_REPORT") else {
            return;
        };
        fs::write(report, keyring_lock_path().unwrap().to_str().unwrap()).unwrap();
    }

    #[cfg(unix)]
    #[test]
    fn keyring_lock_ignores_home_and_temp_overrides() {
        use std::process::Command;

        let dir = tempfile::tempdir().unwrap();
        let expected = keyring_lock_path().unwrap();
        assert!(!expected.starts_with("/tmp"), "{}", expected.display());
        for index in 0..2 {
            let report = dir.path().join(format!("report-{index}"));
            let output = Command::new(std::env::current_exe().unwrap())
                .args([
                    "--exact",
                    "engine::aspect::credential_store::tests::keyring_lock_subprocess_probe",
                    "--nocapture",
                ])
                .env("HOME", dir.path().join(format!("different-home-{index}")))
                .env("TMPDIR", dir.path().join(format!("different-temp-{index}")))
                .env(
                    "XDG_RUNTIME_DIR",
                    dir.path().join(format!("different-runtime-{index}")),
                )
                .env("ASPECT_KEYRING_LOCK_TEST_REPORT", &report)
                .output()
                .unwrap();
            assert!(
                output.status.success(),
                "{}\n{}",
                String::from_utf8_lossy(&output.stdout),
                String::from_utf8_lossy(&output.stderr)
            );
            assert_eq!(
                fs::read_to_string(report).unwrap(),
                expected.to_str().unwrap()
            );
        }
    }

    #[test]
    fn a_no_op_update_leaves_an_unparseable_store_alone() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("credentials.json");
        fs::write(&path, LEGACY_BLOB).unwrap();
        let store = CredentialStore::File(path.clone());
        store
            .update(|map: &mut HashMap<String, FakeEntry>| {
                map.remove("absent");
                Ok(())
            })
            .unwrap();
        assert_eq!(fs::read_to_string(&path).unwrap(), LEGACY_BLOB);
    }

    #[test]
    fn a_no_op_update_creates_no_credentials_file() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("credentials.json");
        CredentialStore::File(path.clone())
            .update(|_: &mut HashMap<String, String>| Ok(()))
            .unwrap();
        assert!(!path.exists());
    }

    #[test]
    fn clear_does_not_need_a_readable_store() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("credentials.json");
        fs::write(&path, [0xff, 0xfe, 0x00]).unwrap();
        let store = CredentialStore::File(path.clone());
        assert!(store.load_all::<String>().is_err());
        store.clear().unwrap();
        assert!(store.load_all::<String>().unwrap().is_empty());
    }

    /// A writable credentials file in a directory the process can't write: no
    /// temp file can be made next to it, so it is written in place.
    #[cfg(unix)]
    #[test]
    fn a_file_in_an_unwritable_directory_is_written_in_place() {
        use std::os::unix::fs::PermissionsExt;
        // SAFETY: geteuid has no preconditions.
        if unsafe { nix::libc::geteuid() } == 0 {
            return; // root writes regardless of directory permissions
        }
        let dir = tempfile::tempdir().unwrap();
        let sealed = dir.path().join("sealed");
        fs::create_dir(&sealed).unwrap();
        let path = sealed.join("credentials.json");
        file_save_all(&path, &HashMap::from([("session".to_string(), "original")])).unwrap();
        fs::set_permissions(&sealed, fs::Permissions::from_mode(0o500)).unwrap();
        let saved = file_save_all(&path, &HashMap::from([("session".to_string(), "rotated")]));
        fs::set_permissions(&sealed, fs::Permissions::from_mode(0o700)).unwrap();
        saved.unwrap();
        let back: HashMap<String, String> = file_load_all(&path).unwrap();
        assert_eq!(back["session"], "rotated");
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

    #[derive(serde::Serialize, serde::Deserialize)]
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
