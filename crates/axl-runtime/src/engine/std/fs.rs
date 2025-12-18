use allocative::Allocative;
use derive_more::Display;
use starlark::environment::Methods;
use starlark::environment::MethodsBuilder;
use starlark::environment::MethodsStatic;
use starlark::values::list::UnpackList;
use starlark::values::none::NoneType;
use starlark::values::ValueOfUnchecked;
use std::fs;
use std::time::UNIX_EPOCH;

use starlark::starlark_module;
use starlark::starlark_simple_value;
use starlark::values;
use starlark::values::starlark_value;
use starlark::values::Heap;
use starlark::values::NoSerialize;
use starlark::values::ProvidesStaticType;
use starlark::values::StringValue;
use starlark::values::Trace;

#[derive(Debug, Clone, ProvidesStaticType, Display, Trace, NoSerialize, Allocative)]
#[display("<fs.DirEntry path:{path} is_dir:{is_dir} is_file:{is_file}>")]
pub struct DirEntry<'v> {
    path: StringValue<'v>,
    is_file: values::Value<'v>,
    is_dir: values::Value<'v>,
}

#[starlark_value(type = "fs.DirEntry")]
impl<'v> values::StarlarkValue<'v> for DirEntry<'v> {
    fn get_attr(&self, attr: &str, _: &Heap) -> Option<values::Value<'v>> {
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
    fn alloc_value(self, heap: &'v values::Heap) -> values::Value<'v> {
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
#[display("<fs.Metadata is_dir:{is_dir} is_file:{is_file} is_symlink:{is_symlink} size:{size}>")]
pub struct Metadata<'v> {
    is_dir: values::Value<'v>,
    is_file: values::Value<'v>,
    is_symlink: values::Value<'v>,
    size: values::Value<'v>,
    modified: values::Value<'v>,
    accessed: values::Value<'v>,
    created: values::Value<'v>,
    readonly: values::Value<'v>,
}

#[starlark_value(type = "fs.Metadata")]
impl<'v> values::StarlarkValue<'v> for Metadata<'v> {
    fn get_attr(&self, attr: &str, _: &Heap) -> Option<values::Value<'v>> {
        match attr {
            "is_dir" => Some(self.is_dir),
            "is_file" => Some(self.is_file),
            "is_symlink" => Some(self.is_symlink),
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
            "size".into(),
            "modified".into(),
            "accessed".into(),
            "created".into(),
            "readonly".into(),
        ]
    }
}

impl<'v> values::AllocValue<'v> for Metadata<'v> {
    fn alloc_value(self, heap: &'v values::Heap) -> values::Value<'v> {
        heap.alloc_complex(self)
    }
}

#[derive(Debug, Display, ProvidesStaticType, NoSerialize, Allocative)]
#[display("<fs.Metadata>")]
pub struct FrozenMetadata {
    is_dir: values::FrozenValue,
    is_file: values::FrozenValue,
    is_symlink: values::FrozenValue,
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
    fn metadata<'v>(
        #[allow(unused)] this: values::Value<'v>,
        #[starlark(require = pos)] path: values::StringValue,
        heap: &'v Heap,
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
        heap: &'v Heap,
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
        heap: &'v Heap,
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
        heap: &'v Heap,
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

    // TODO: add set_permissions

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
    fn symlink_metadata<'v>(
        #[allow(unused)] this: values::Value<'v>,
        #[starlark(require = pos)] path: values::StringValue,
        heap: &'v Heap,
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
}

fn marshal_metadata<'v>(m: &fs::Metadata, heap: &'v Heap) -> Metadata<'v> {
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
    Metadata {
        is_dir: heap.alloc(file_type.is_dir()),
        is_file: heap.alloc(file_type.is_file()),
        is_symlink: heap.alloc(file_type.is_symlink()),
        size: heap.alloc(m.len()),
        modified,
        accessed,
        created,
        readonly: heap.alloc(permissions.readonly()),
    }
}
