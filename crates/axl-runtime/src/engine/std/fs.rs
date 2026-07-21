use allocative::Allocative;
use derive_more::Display;
use starlark::environment::Methods;
use starlark::environment::MethodsBuilder;
use starlark::environment::MethodsStatic;
use starlark::values::ValueOfUnchecked;
use starlark::values::list::UnpackList;
use starlark::values::none::{NoneOr, NoneType};
use std::collections::HashMap;
use std::fs;
use std::io::Write;
use std::sync::{Arc, Condvar, Mutex, OnceLock};
use std::time::{Duration, UNIX_EPOCH};

use super::stream;

use starlark::starlark_module;
use starlark::starlark_simple_value;
use starlark::values;
use starlark::values::Heap;
use starlark::values::NoSerialize;
use starlark::values::ProvidesStaticType;
use starlark::values::StringValue;
use starlark::values::Trace;
use starlark::values::starlark_value;

#[derive(Debug, Clone, ProvidesStaticType, Display, Trace, NoSerialize, Allocative)]
#[display("<fs.DirEntry path:{path} is_dir:{is_dir} is_file:{is_file}>")]
pub struct DirEntry<'v> {
    path: StringValue<'v>,
    is_file: values::Value<'v>,
    is_dir: values::Value<'v>,
}

#[starlark_value(type = "fs.DirEntry")]
impl<'v> values::StarlarkValue<'v> for DirEntry<'v> {
    fn get_attr(&self, attr: &str, _: Heap<'v>) -> Option<values::Value<'v>> {
        match attr {
            "path" => Some(self.path.to_value()),
            "is_file" => Some(self.is_file),
            "is_dir" => Some(self.is_dir),
            _ => None,
        }
    }
    fn dir_attr(&self) -> Vec<String> {
        vec!["path".into(), "is_file".into(), "is_dir".into()]
    }
}

impl<'v> values::AllocValue<'v> for DirEntry<'v> {
    fn alloc_value(self, heap: values::Heap<'v>) -> values::Value<'v> {
        heap.alloc_complex(self)
    }
}

#[derive(Debug, Display, ProvidesStaticType, NoSerialize, Allocative)]
#[display("<fs.DirEntry>")]
pub struct FrozenDirEntry {
    path: values::FrozenValue,
    is_file: values::FrozenValue,
    is_dir: values::FrozenValue,
}

#[starlark_value(type = "fs.DirEntry")]
impl<'v> values::StarlarkValue<'v> for FrozenDirEntry {
    type Canonical = DirEntry<'v>;
}

impl<'v> values::Freeze for DirEntry<'v> {
    type Frozen = FrozenDirEntry;
    fn freeze(self, freezer: &values::Freezer) -> values::FreezeResult<Self::Frozen> {
        Ok(FrozenDirEntry {
            path: freezer.freeze(self.path.to_value())?,
            is_dir: freezer.freeze(self.is_dir)?,
            is_file: freezer.freeze(self.is_file)?,
        })
    }
}

starlark_simple_value!(FrozenDirEntry);

#[derive(Debug, Clone, ProvidesStaticType, Display, Trace, NoSerialize, Allocative)]
#[display(
    "<fs.Metadata is_dir:{is_dir} is_file:{is_file} is_symlink:{is_symlink} executable:{executable} size:{size}>"
)]
pub struct Metadata<'v> {
    is_dir: values::Value<'v>,
    is_file: values::Value<'v>,
    is_symlink: values::Value<'v>,
    executable: values::Value<'v>,
    size: values::Value<'v>,
    modified: values::Value<'v>,
    accessed: values::Value<'v>,
    created: values::Value<'v>,
    readonly: values::Value<'v>,
}

#[starlark_value(type = "fs.Metadata")]
impl<'v> values::StarlarkValue<'v> for Metadata<'v> {
    fn get_attr(&self, attr: &str, _: Heap<'v>) -> Option<values::Value<'v>> {
        match attr {
            "is_dir" => Some(self.is_dir),
            "is_file" => Some(self.is_file),
            "is_symlink" => Some(self.is_symlink),
            "executable" => Some(self.executable),
            "size" => Some(self.size),
            "modified" => Some(self.modified),
            "accessed" => Some(self.accessed),
            "created" => Some(self.created),
            "readonly" => Some(self.readonly),
            _ => None,
        }
    }
    fn dir_attr(&self) -> Vec<String> {
        vec![
            "is_dir".into(),
            "is_file".into(),
            "is_symlink".into(),
            "executable".into(),
            "size".into(),
            "modified".into(),
            "accessed".into(),
            "created".into(),
            "readonly".into(),
        ]
    }
}

impl<'v> values::AllocValue<'v> for Metadata<'v> {
    fn alloc_value(self, heap: values::Heap<'v>) -> values::Value<'v> {
        heap.alloc_complex(self)
    }
}

#[derive(Debug, Display, ProvidesStaticType, NoSerialize, Allocative)]
#[display("<fs.Metadata>")]
pub struct FrozenMetadata {
    is_dir: values::FrozenValue,
    is_file: values::FrozenValue,
    is_symlink: values::FrozenValue,
    executable: values::FrozenValue,
    size: values::FrozenValue,
    modified: values::FrozenValue,
    accessed: values::FrozenValue,
    created: values::FrozenValue,
    readonly: values::FrozenValue,
}

#[starlark_value(type = "fs.Metadata")]
impl<'v> values::StarlarkValue<'v> for FrozenMetadata {
    type Canonical = Metadata<'v>;
}

impl<'v> values::Freeze for Metadata<'v> {
    type Frozen = FrozenMetadata;
    fn freeze(self, freezer: &values::Freezer) -> values::FreezeResult<Self::Frozen> {
        Ok(FrozenMetadata {
            is_dir: freezer.freeze(self.is_dir)?,
            is_file: freezer.freeze(self.is_file)?,
            is_symlink: freezer.freeze(self.is_symlink)?,
            executable: freezer.freeze(self.executable)?,
            size: freezer.freeze(self.size)?,
            modified: freezer.freeze(self.modified)?,
            accessed: freezer.freeze(self.accessed)?,
            created: freezer.freeze(self.created)?,
            readonly: freezer.freeze(self.readonly)?,
        })
    }
}

starlark_simple_value!(FrozenMetadata);

#[derive(Debug, Display, ProvidesStaticType, NoSerialize, Allocative)]
#[display("<std.Filesystem>")]
pub struct Filesystem {}

impl Filesystem {
    pub fn new() -> Self {
        Self {}
    }
}

#[starlark_value(type = "std.Filesystem")]
impl<'v> values::StarlarkValue<'v> for Filesystem {
    fn get_methods() -> Option<&'static Methods> {
        static RES: MethodsStatic = MethodsStatic::new();
        RES.methods(filesystem_methods)
    }
}

starlark_simple_value!(Filesystem);

/// A background read whose completion callers poll for. `state` is `None`
/// while the reader thread is still running; `Some(result)` once it finished
/// (`result` is `None` on any I/O/UTF-8 error, `Some(content)` on success).
struct ReadProbe {
    state: Mutex<Option<Option<String>>>,
    done: Condvar,
}

/// In-flight [`ReadProbe`]s keyed by path. Entries are removed when a poll
/// consumes a completed probe; a probe whose read never returns (e.g. a FUSE
/// request the filesystem daemon never answers) leaves its entry — and its
/// stuck reader thread — in place for the life of the process, which is what
/// bounds the leak to one thread per distinct stuck path.
static READ_PROBES: OnceLock<Mutex<HashMap<String, Arc<ReadProbe>>>> = OnceLock::new();

/// Poll a coalesced background read of `path` (see `poll_read_to_string` for
/// the contract). Only the call that spawns the probe blocks (up to
/// `timeout`); later polls for the same path return its state immediately.
fn poll_read(path: &str, timeout: Duration) -> Option<String> {
    let registry = READ_PROBES.get_or_init(Default::default);
    let (probe, spawned_here) = {
        let mut reg = registry.lock().unwrap();
        match reg.get(path) {
            Some(probe) => (probe.clone(), false),
            None => {
                let probe = Arc::new(ReadProbe {
                    state: Mutex::new(None),
                    done: Condvar::new(),
                });
                reg.insert(path.to_string(), probe.clone());
                let thread_probe = probe.clone();
                let thread_path = path.to_string();
                let spawned = std::thread::Builder::new()
                    .name("fs-read-probe".to_string())
                    .spawn(move || {
                        let result = fs::read_to_string(&thread_path).ok();
                        *thread_probe.state.lock().unwrap() = Some(result);
                        thread_probe.done.notify_all();
                    });
                if spawned.is_err() {
                    *probe.state.lock().unwrap() = Some(None);
                }
                (probe, true)
            }
        }
    };

    let mut state = probe.state.lock().unwrap();
    if spawned_here && state.is_none() {
        state = probe
            .done
            .wait_timeout_while(state, timeout, |s| s.is_none())
            .unwrap()
            .0;
    }
    let result = state.clone();
    drop(state);
    match result {
        Some(result) => {
            registry.lock().unwrap().remove(path);
            result
        }
        None => None,
    }
}

#[starlark_module]
pub(crate) fn filesystem_methods(registry: &mut MethodsBuilder) {
    /// Copies the contents of one file to another. This function will also copy the permission bits of the original file to the destination file.
    ///
    /// This function will overwrite the contents of to.
    /// Note that if from and to both point to the same file, then the file will likely get truncated by this operation.
    /// On success, the total number of bytes copied is returned and it is equal to the length of the to file as reported by metadata.
    ///
    /// This function will return an error in the following situations, but is not limited to just these cases:
    /// - from is neither a regular file nor a symlink to a regular file.
    /// - from does not exist.
    /// - The current process does not have the permission rights to read from or write to.
    /// - The parent directory of to doesn’t exist.
    fn copy<'v>(
        #[allow(unused)] this: values::Value<'v>,
        #[starlark(require = pos)] from: values::StringValue,
        #[starlark(require = pos)] to: values::StringValue,
    ) -> anyhow::Result<u64> {
        Ok(fs::copy(from.as_str(), to.as_str())?)
    }

    /// Creates a new, empty directory at the provided path.
    ///
    /// This function will return an error in the following situations, but is not limited to just these cases:
    /// - User lacks permissions to create directory at path.
    /// - A parent of the given path doesn’t exist. (To create a directory and all its missing parents at the same time, use the create_dir_all function.)
    /// - path already exists.
    ///
    /// NOTE: If a parent of the given path doesn’t exist, this function will return an error. To create a directory and all its missing parents at the same time, use the create_dir_all function.
    fn create_dir<'v>(
        #[allow(unused)] this: values::Value<'v>,
        #[starlark(require = pos)] path: values::StringValue,
    ) -> anyhow::Result<NoneType> {
        fs::create_dir(path.as_str())?;
        Ok(NoneType)
    }

    /// Recursively create a directory and all of its parent components if they are missing.
    ///
    /// This function is not atomic. If it returns an error, any parent components it was able to create will remain.
    /// If the empty path is passed to this function, it always succeeds without creating any directories.
    ///
    /// The function will return an error if any directory specified in path does not exist and could not be created. There may be other error conditions; see create_dir for specifics.
    fn create_dir_all<'v>(
        #[allow(unused)] this: values::Value<'v>,
        #[starlark(require = pos)] path: values::StringValue,
    ) -> anyhow::Result<NoneType> {
        fs::create_dir_all(path.as_str())?;
        Ok(NoneType)
    }

    /// Creates a new temporary directory with a unique name and returns its path.
    ///
    /// The directory is created inside `parent` (defaults to the system temp dir).
    /// The caller is responsible for removing it when done.
    fn mkdtemp<'v>(
        #[allow(unused)] this: values::Value<'v>,
        #[starlark(require = named, default = "")] prefix: &str,
        #[starlark(require = named, default = "")] parent: &str,
        heap: Heap<'v>,
    ) -> anyhow::Result<values::StringValue<'v>> {
        let base = if parent.is_empty() {
            std::env::temp_dir()
        } else {
            std::path::PathBuf::from(parent)
        };
        let prefix = if prefix.is_empty() { "axl-" } else { prefix };
        // Use uuid v4 for a collision-free unique suffix.
        let dir = base.join(format!("{}{}", prefix, uuid::Uuid::new_v4()));
        fs::create_dir_all(&dir)?;
        let path = dir
            .to_str()
            .ok_or_else(|| anyhow::anyhow!("temp dir path is non-UTF-8"))?;
        Ok(heap.alloc_str(path))
    }

    /// Creates a new hard link on the filesystem.
    ///
    /// The link path will be a link pointing to the original path. Note that systems often require these two paths to both be located on the same filesystem.
    ///
    /// If original names a symbolic link, it is platform-specific whether the symbolic link is followed. On platforms where it’s possible to not follow it, it is not followed, and the created hard link points to the symbolic link itself.
    ///
    /// This function will return an error in the following situations, but is not limited to just these cases:
    /// - The original path is not a file or doesn’t exist.
    /// - The ‘link’ path already exists.
    fn hard_link<'v>(
        #[allow(unused)] this: values::Value<'v>,
        #[starlark(require = pos)] original: values::StringValue,
        #[starlark(require = pos)] link: values::StringValue,
    ) -> anyhow::Result<NoneType> {
        fs::hard_link(original.as_str(), link.as_str())?;
        Ok(NoneType)
    }

    /// Returns `true` if the path points at an existing entity.
    ///
    /// This function will traverse symbolic links to query information about the
    /// destination file. In case of broken symbolic links this will return `false`.
    ///
    /// Note that while this avoids some pitfalls of the `exists()` method, it still can not
    /// prevent time-of-check to time-of-use (TOCTOU) bugs. You should only use it in scenarios
    /// where those bugs are not an issue.
    fn exists<'v>(
        #[allow(unused)] this: values::Value<'v>,
        #[starlark(require = pos)] path: values::StringValue,
    ) -> anyhow::Result<bool> {
        Ok(fs::exists(path.as_str())?)
    }

    /// Returns true if this path is for a directory.
    ///
    /// This function will return an error in the following situations, but is not limited to just these cases:
    /// - The user lacks permissions to perform metadata call on path.
    /// - path does not exist.
    fn is_dir<'v>(
        #[allow(unused)] this: values::Value<'v>,
        #[starlark(require = pos)] path: values::StringValue,
    ) -> anyhow::Result<bool> {
        let metadata = fs::metadata(path.as_str())?;
        Ok(metadata.is_dir())
    }

    /// Returns true if this path is for a regular file.
    ///
    /// This function will return an error in the following situations, but is not limited to just these cases:
    /// - The user lacks permissions to perform metadata call on path.
    /// - path does not exist.
    fn is_file<'v>(
        #[allow(unused)] this: values::Value<'v>,
        #[starlark(require = pos)] path: values::StringValue,
    ) -> anyhow::Result<bool> {
        println!(
            r#"Deprecated: is_file is deprecated and will be removed in a future version of AXL.
            Use `fs.metadata().is_file` instead."#
        );
        let metadata = fs::metadata(path.as_str())?;
        Ok(metadata.is_file())
    }

    /// Returns the metadata about the given file or directory.
    ///
    /// This function will return an error in the following situations, but is not limited to just these cases:
    /// - The user lacks permissions to perform metadata call on path.
    /// - path does not exist.
    ///
    /// The modified, accessed, created fields of the Metadata result might not be available on all platforms, and will
    /// be set to None on platforms where they is not available.
    /// The executable field reflects the Unix execute bit; it is always false on non-Unix platforms.
    fn metadata<'v>(
        #[allow(unused)] this: values::Value<'v>,
        #[starlark(require = pos)] path: values::StringValue,
        heap: Heap<'v>,
    ) -> anyhow::Result<Metadata<'v>> {
        let m = fs::metadata(path.as_str())?;
        Ok(marshal_metadata(&m, heap))
    }

    /// Returns an iterator over the entries within a directory.
    ///
    /// This function will return an error in the following situations, but is not limited to just these cases:
    /// - The provided path doesn’t exist.
    /// - The process lacks permissions to view the contents.
    /// - The path points at a non-directory file.
    fn read_dir<'v>(
        #[allow(unused)] this: values::Value<'v>,
        #[starlark(require = pos)] path: values::StringValue,
        heap: Heap<'v>,
    ) -> anyhow::Result<ValueOfUnchecked<'v, UnpackList<DirEntry<'v>>>> {
        let metadata = fs::read_dir(path.as_str())?;
        Ok(heap
            .alloc_typed_unchecked(values::list::AllocList(metadata.map(|entry| {
                let entry = entry.unwrap();
                let file_type = entry.file_type().unwrap();
                DirEntry {
                    path: heap.alloc_str(entry.file_name().to_str().unwrap()),
                    // TODO: implement a filetype and expose that.
                    is_dir: heap.alloc(file_type.is_dir()),
                    is_file: heap.alloc(file_type.is_file()),
                }
                // TODO: return a iterator of DirEntry type.
            })))
            .cast())
    }

    /// Reads a symbolic link, returning the file that the link points to.
    ///
    /// This function will return an error in the following situations, but is not limited to just these cases:
    /// - path is not a symbolic link.
    /// - path does not exist.
    fn read_link<'v>(
        #[allow(unused)] this: values::Value<'v>,
        #[starlark(require = pos)] path: values::StringValue,
        heap: Heap<'v>,
    ) -> anyhow::Result<values::StringValue<'v>> {
        let r = fs::read_link(path.as_str())
            .map(|f| heap.alloc_str(&f.as_os_str().to_string_lossy().to_string()))?;
        Ok(r)
    }

    /// Reads the entire contents of a file into a string.
    ///
    /// This function will return an error under a number of different circumstances. Some of these error conditions are:
    /// - The specified file does not exist.
    /// - The user lacks permission to get the specified access rights for the file.
    /// - The user lacks permission to open one of the directory components of the specified path.
    fn read_to_string<'v>(
        #[allow(unused)] this: values::Value<'v>,
        #[starlark(require = pos)] path: values::StringValue,
        heap: Heap<'v>,
    ) -> anyhow::Result<values::StringValue<'v>> {
        let r = fs::read_to_string(path.as_str()).map(|f| heap.alloc_str(f.as_str()))?;
        Ok(r)
    }

    /// Removes an empty directory.
    ///
    /// If you want to remove a directory that is not empty, as well as all
    /// of its contents recursively, consider using remove_dir_all instead.
    ///
    /// This function will return an error in the following situations, but is not limited to just these cases:
    /// - path doesn’t exist.
    /// - path isn’t a directory.
    /// - The user lacks permissions to remove the directory at the provided path.
    /// - The directory isn’t empty.
    fn remove_dir<'v>(
        #[allow(unused)] this: values::Value<'v>,
        #[starlark(require = pos)] path: values::StringValue,
    ) -> anyhow::Result<NoneType> {
        fs::remove_dir(path.as_str())?;
        Ok(NoneType)
    }

    /// Removes a directory at this path, after removing all its contents. Use carefully!
    ///
    /// This function does not follow symbolic links and it will simply remove the symbolic link itself.
    ///
    /// See remove_file and remove_dir for possible errors.
    fn remove_dir_all<'v>(
        #[allow(unused)] this: values::Value<'v>,
        #[starlark(require = pos)] path: values::StringValue,
    ) -> anyhow::Result<NoneType> {
        fs::remove_dir_all(path.as_str())?;
        Ok(NoneType)
    }

    /// Removes a file from the filesystem.
    ///
    /// Note that there is no guarantee that the file is immediately deleted
    /// (e.g., depending on platform, other open file descriptors may prevent immediate removal).
    ///
    /// This function will return an error in the following situations, but is not limited to just these cases:
    /// - path points to a directory.
    /// - The file doesn’t exist.
    /// - The user lacks permissions to remove the file.
    fn remove_file<'v>(
        #[allow(unused)] this: values::Value<'v>,
        #[starlark(require = pos)] path: values::StringValue,
    ) -> anyhow::Result<NoneType> {
        fs::remove_file(path.as_str())?;
        Ok(NoneType)
    }

    /// Renames a file or directory to a new name, replacing the original file if to already exists.
    ///
    /// This will not work if the new name is on a different mount point.
    ///
    /// This function will return an error in the following situations, but is not limited to just these cases:
    /// - from does not exist.
    /// - The user lacks permissions to view contents.
    /// - from and to are on separate filesystems.
    fn rename<'v>(
        #[allow(unused)] this: values::Value<'v>,
        #[starlark(require = pos)] from: values::StringValue,
        #[starlark(require = pos)] to: values::StringValue,
    ) -> anyhow::Result<NoneType> {
        fs::rename(from.as_str(), to.as_str())?;
        Ok(NoneType)
    }

    /// Sets the Unix permission bits of a file to `mode` (e.g. `0o755`).
    ///
    /// On non-Unix platforms this is a no-op. Useful when writing a file with
    /// `fs.write` (which uses the default umask) but the result must be
    /// executable, e.g. a generated shell script or tool wrapper.
    ///
    /// Returns an error if `path` does not exist or the user lacks permission to
    /// change its mode (not an exhaustive list).
    fn set_permissions<'v>(
        #[allow(unused)] this: values::Value<'v>,
        #[starlark(require = pos)] path: values::StringValue,
        #[starlark(require = pos)] mode: u32,
    ) -> anyhow::Result<NoneType> {
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            fs::set_permissions(path.as_str(), fs::Permissions::from_mode(mode))?;
        }
        #[cfg(not(unix))]
        {
            let _ = (path, mode);
        }
        Ok(NoneType)
    }

    // NB: Don't add deprecated soft_link (https://doc.rust-lang.org/std/fs/fn.soft_link.html);
    // instead add std.os.unix.fs.symlink to follow Rust's convention of not adding
    // os specific function to non-os specific std lib location.

    /// Queries the metadata about a file without following symlinks.
    ///
    /// This function will return an error in the following situations, but is not limited to just these cases:
    /// - The user lacks permissions to perform metadata call on path.
    /// - path does not exist.
    ///
    /// The modified, accessed, created fields of the Metadata result might not be available on all platforms, and will
    /// be set to None on platforms where they is not available.
    /// The executable field reflects the Unix execute bit; it is always false on non-Unix platforms.
    fn symlink_metadata<'v>(
        #[allow(unused)] this: values::Value<'v>,
        #[starlark(require = pos)] path: values::StringValue,
        heap: Heap<'v>,
    ) -> anyhow::Result<Metadata<'v>> {
        let m = fs::symlink_metadata(path.as_str())?;
        Ok(marshal_metadata(&m, heap))
    }

    /// Writes a string as the entire contents of a file.
    ///
    /// This function will create a file if it does not exist, and will entirely replace its contents if it does.
    /// Depending on the platform, this function may fail if the full directory path does not exist.
    /// This is a convenience function for using fs.create and [write_all] with fewer imports.
    fn write<'v>(
        #[allow(unused)] this: values::Value<'v>,
        #[starlark(require = pos)] path: values::StringValue,
        #[starlark(require = pos)] content: values::StringValue,
    ) -> anyhow::Result<NoneType> {
        fs::write(path.as_str(), content.as_str())?;
        Ok(NoneType)
    }

    /// Appends a string to the end of a file, creating it if it does not exist.
    ///
    /// Opens the file with `OpenOptions::append(true)` so concurrent writers
    /// see their bytes interleaved at record boundaries rather than racing.
    /// POSIX `O_APPEND` is atomic for writes ≤ `PIPE_BUF`, which covers the
    /// short single-line records this is designed for (the
    /// `runner_job_history` lines fit comfortably). The parent directory
    /// must exist; this function will not create intermediate directories.
    ///
    /// Returns `True` on success and `False` on any I/O error (parent
    /// directory missing, permissions denied, target is a directory, etc.).
    /// Errors are swallowed because the primary caller is the runner job
    /// history hook, where a write failure must never fail the task.
    fn try_append<'v>(
        #[allow(unused)] this: values::Value<'v>,
        #[starlark(require = pos)] path: values::StringValue,
        #[starlark(require = pos)] content: values::StringValue,
    ) -> anyhow::Result<bool> {
        let result = fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(path.as_str())
            .and_then(|mut f| f.write_all(content.as_str().as_bytes()));
        Ok(result.is_ok())
    }

    /// Reads the entire contents of a file into a string, or returns `""`
    /// on any I/O error (missing file, permission denied, non-UTF-8 content,
    /// transient read failure). Companion to `try_append`: the silent
    /// fall-through lets callers handle never-fail invariants without
    /// try/except — e.g. the runner job history dedup read, where a
    /// failed read must degrade to "assume empty file" rather than fail
    /// the task.
    fn try_read_to_string<'v>(
        #[allow(unused)] this: values::Value<'v>,
        #[starlark(require = pos)] path: values::StringValue,
        heap: Heap<'v>,
    ) -> anyhow::Result<values::StringValue<'v>> {
        let s = fs::read_to_string(path.as_str()).unwrap_or_default();
        Ok(heap.alloc_str(s.as_str()))
    }

    /// Reads the entire contents of a file into a string without ever
    /// blocking the caller for more than `timeout_ms`, or returns `None`
    /// when the contents aren't available yet.
    ///
    /// Built for polling files on network-backed filesystems (e.g. a
    /// bb_clientd FUSE mount materializing remote-cache blobs), where a read
    /// can block indefinitely — a plain `read_to_string` would wedge the
    /// task. The read runs on a background thread; the first call for a path
    /// waits up to `timeout_ms` for it, and while that read is still in
    /// flight, subsequent calls for the same path return `None` immediately
    /// rather than starting another read. Whenever the read eventually
    /// completes, the next call returns its result: the file contents, or
    /// `None` on any I/O/UTF-8 error (including a missing file).
    ///
    /// Callers are expected to poll until content or a caller-side deadline:
    /// `None` always means "not readable yet", never "empty file" (an empty
    /// readable file returns `""`).
    fn poll_read_to_string<'v>(
        #[allow(unused)] this: values::Value<'v>,
        #[starlark(require = pos)] path: values::StringValue,
        #[starlark(require = pos)] timeout_ms: i32,
        heap: Heap<'v>,
    ) -> anyhow::Result<NoneOr<values::StringValue<'v>>> {
        let timeout = Duration::from_millis(timeout_ms.max(0) as u64);
        Ok(match poll_read(path.as_str(), timeout) {
            Some(content) => NoneOr::Other(heap.alloc_str(content.as_str())),
            None => NoneOr::None,
        })
    }

    /// Reads at most `max_bytes` bytes from the start of a file into a string,
    /// or returns `""` on any I/O error (missing file, permission denied,
    /// transient failure). Like `try_read_to_string`, the silent fall-through
    /// lets callers degrade rather than fail the task.
    ///
    /// Only the capped prefix is read into memory — never the whole file — so a
    /// multi-MB log can be summarized without spiking the heap. Non-UTF-8 bytes
    /// are replaced (lossy), and the prefix may split a multi-byte sequence at
    /// the cap; that final partial char is dropped rather than emitted as `\u{FFFD}`.
    fn try_read_to_string_capped<'v>(
        #[allow(unused)] this: values::Value<'v>,
        #[starlark(require = pos)] path: values::StringValue,
        #[starlark(require = pos)] max_bytes: i32,
        heap: Heap<'v>,
    ) -> anyhow::Result<values::StringValue<'v>> {
        use std::io::Read;
        if max_bytes <= 0 {
            return Ok(heap.alloc_str(""));
        }
        let cap = max_bytes as u64;
        let s = match fs::File::open(path.as_str()) {
            Ok(f) => {
                let mut buf = Vec::new();
                if f.take(cap).read_to_end(&mut buf).is_ok() {
                    match String::from_utf8(buf) {
                        Ok(valid) => valid,
                        // Lossy decode, then drop a trailing replacement char that
                        // a cap-split multi-byte sequence would have produced.
                        Err(e) => {
                            let valid_up_to = e.utf8_error().valid_up_to();
                            let bytes = e.into_bytes();
                            String::from_utf8_lossy(&bytes[..valid_up_to]).into_owned()
                        }
                    }
                } else {
                    String::new()
                }
            }
            Err(_) => String::new(),
        };
        Ok(heap.alloc_str(s.as_str()))
    }

    /// Opens a file for reading and returns it as a readable stream.
    ///
    /// The returned stream can be passed directly as the `data` argument to
    /// `ctx.http().post()` or `ctx.http().put()` for streaming uploads, or
    /// iterated over / read directly.
    fn open<'v>(
        #[allow(unused)] this: values::Value<'v>,
        #[starlark(require = pos)] path: values::StringValue,
    ) -> anyhow::Result<stream::Readable> {
        let file = fs::File::open(path.as_str())?;
        Ok(stream::Readable::File(Arc::new(Mutex::new(file))))
    }

    /// Creates (or truncates) a file for writing and returns it as a writable stream.
    ///
    /// Mirrors `std::fs::File::create` — the file is opened write-only and
    /// truncated to zero length, or created if it does not exist.
    fn create<'v>(
        #[allow(unused)] this: values::Value<'v>,
        #[starlark(require = pos)] path: values::StringValue,
    ) -> anyhow::Result<stream::Writable> {
        let file = fs::File::create(path.as_str())?;
        Ok(stream::Writable::from(file))
    }
}

fn marshal_metadata<'v>(m: &fs::Metadata, heap: Heap<'v>) -> Metadata<'v> {
    let file_type = m.file_type();
    let modified = match m.modified() {
        Ok(t) => match t.duration_since(UNIX_EPOCH) {
            Ok(d) => heap.alloc(d.as_micros() as u64),
            Err(_) => heap.alloc(NoneType),
        },
        Err(_) => heap.alloc(NoneType),
    };
    let accessed = match m.accessed() {
        Ok(t) => match t.duration_since(UNIX_EPOCH) {
            Ok(d) => heap.alloc(d.as_micros() as u64),
            Err(_) => heap.alloc(NoneType),
        },
        Err(_) => heap.alloc(NoneType),
    };
    let created = match m.created() {
        Ok(t) => match t.duration_since(UNIX_EPOCH) {
            Ok(d) => heap.alloc(d.as_micros() as u64),
            Err(_) => heap.alloc(NoneType),
        },
        Err(_) => heap.alloc(NoneType),
    };
    let permissions = m.permissions();
    #[cfg(unix)]
    let executable = {
        use std::os::unix::fs::PermissionsExt;
        heap.alloc(permissions.mode() & 0o111 != 0)
    };
    #[cfg(not(unix))]
    let executable = heap.alloc(false);
    Metadata {
        is_dir: heap.alloc(file_type.is_dir()),
        is_file: heap.alloc(file_type.is_file()),
        is_symlink: heap.alloc(file_type.is_symlink()),
        executable,
        size: heap.alloc(m.len()),
        modified,
        accessed,
        created,
        readonly: heap.alloc(permissions.readonly()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn poll_read_returns_content_for_readable_file() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("ok.txt");
        fs::write(&path, "hello").unwrap();
        let got = poll_read(path.to_str().unwrap(), Duration::from_secs(5));
        assert_eq!(got.as_deref(), Some("hello"));
        // Consumed probes are removed so a later poll re-reads fresh content.
        fs::write(&path, "world").unwrap();
        let got = poll_read(path.to_str().unwrap(), Duration::from_secs(5));
        assert_eq!(got.as_deref(), Some("world"));
    }

    #[test]
    fn poll_read_distinguishes_empty_from_unreadable() {
        let dir = tempfile::tempdir().unwrap();
        let empty = dir.path().join("empty.txt");
        fs::write(&empty, "").unwrap();
        assert_eq!(
            poll_read(empty.to_str().unwrap(), Duration::from_secs(5)).as_deref(),
            Some("")
        );
        let missing = dir.path().join("missing.txt");
        assert_eq!(
            poll_read(missing.to_str().unwrap(), Duration::from_secs(5)),
            None
        );
    }

    /// A FIFO with no writer blocks `open()` indefinitely — the same shape as
    /// a FUSE read the daemon never answers. The first poll must give up at
    /// its timeout, later polls must return immediately while the probe is
    /// still stuck, and once the "blob" arrives (a writer opens the FIFO) a
    /// later poll must deliver the content.
    #[cfg(unix)]
    #[test]
    fn poll_read_never_blocks_on_a_stuck_read() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("stuck.fifo");
        nix::unistd::mkfifo(&path, nix::sys::stat::Mode::from_bits(0o644).unwrap()).unwrap();
        let p = path.to_str().unwrap();

        let started = std::time::Instant::now();
        assert_eq!(poll_read(p, Duration::from_millis(200)), None);
        assert!(
            started.elapsed() < Duration::from_secs(5),
            "first poll must return at its timeout"
        );

        // Probe is in flight: this must not block for another timeout.
        let started = std::time::Instant::now();
        assert_eq!(poll_read(p, Duration::from_secs(60)), None);
        assert!(
            started.elapsed() < Duration::from_secs(5),
            "in-flight poll must return immediately"
        );

        // The "download" lands: the stuck open() completes, EOF ends the read.
        fs::OpenOptions::new()
            .write(true)
            .open(&path)
            .and_then(|mut f| f.write_all(b"landed"))
            .unwrap();

        let deadline = std::time::Instant::now() + Duration::from_secs(10);
        loop {
            match poll_read(p, Duration::from_millis(100)) {
                Some(content) => {
                    assert_eq!(content, "landed");
                    break;
                }
                None => {
                    assert!(
                        std::time::Instant::now() < deadline,
                        "probe never completed after the FIFO writer arrived"
                    );
                    std::thread::sleep(Duration::from_millis(20));
                }
            }
        }
    }
}
