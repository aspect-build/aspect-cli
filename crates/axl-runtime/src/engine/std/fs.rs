use allocative::Allocative;
use derive_more::Display;
use starlark::environment::Methods;
use starlark::environment::MethodsBuilder;
use starlark::environment::MethodsStatic;
use starlark::values::ValueOfUnchecked;
use starlark::values::list::UnpackList;
use starlark::values::none::NoneType;
use std::fs;

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

#[derive(Debug, Display, ProvidesStaticType, NoSerialize, Allocative)]
#[display("<filesystem>")]
pub struct Filesystem {}

impl Filesystem {
    pub fn new() -> Self {
        Self {}
    }
}

#[starlark_value(type = "filesystem")]
impl<'v> values::StarlarkValue<'v> for Filesystem {
    fn get_methods() -> Option<&'static Methods> {
        static RES: MethodsStatic = MethodsStatic::new();
        RES.methods(filesystem_methods)
    }
}

starlark_simple_value!(Filesystem);

#[starlark_module]
pub(crate) fn filesystem_methods(registry: &mut MethodsBuilder) {
    /// Returns true if this path is for a directory.
    fn is_dir<'v>(
        #[allow(unused)] this: values::Value<'v>,
        #[starlark(require = pos)] path: values::StringValue,
    ) -> anyhow::Result<bool> {
        let metadata = fs::metadata(path.as_str())?;
        Ok(metadata.is_dir())
    }

    /// Returns true if this path is for a regular file.
    fn is_file<'v>(
        #[allow(unused)] this: values::Value<'v>,
        #[starlark(require = pos)] path: values::StringValue,
    ) -> anyhow::Result<bool> {
        let metadata = fs::metadata(path.as_str())?;
        Ok(metadata.is_file())
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

    /// Returns an iterator over the entries within a directory.
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

    /// Writes a slice as the entire contents of a file.
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

    fn read_to_string<'v>(
        #[allow(unused)] this: values::Value<'v>,
        #[starlark(require = pos)] path: values::StringValue,
        heap: &'v Heap,
    ) -> anyhow::Result<values::StringValue<'v>> {
        let r = fs::read_to_string(path.as_str()).map(|f| heap.alloc_str(f.as_str()))?;
        Ok(r)
    }
}
