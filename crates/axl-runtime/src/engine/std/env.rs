use allocative::Allocative;
use derive_more::Display;
use starlark::environment::{Methods, MethodsBuilder, MethodsStatic};
use starlark::eval::Evaluator;
use starlark::values::list::{AllocList, UnpackList};
use starlark::values::none::NoneOr;
use starlark::values::tuple::{AllocTuple, UnpackTuple};
use starlark::values::{starlark_value, StarlarkValue};
use starlark::values::{Heap, NoSerialize, ProvidesStaticType, ValueOfUnchecked};
use starlark::{starlark_module, starlark_simple_value, values};

use crate::engine::store::AxlStore;

#[derive(Clone, Debug, ProvidesStaticType, NoSerialize, Allocative, Display)]
#[display("<std.Env>")]
pub struct Env {}

impl Env {
    pub fn new() -> Self {
        Self {}
    }
}

/// Documentation here
#[starlark_value(type = "std.Env")]
impl<'v> StarlarkValue<'v> for Env {
    fn get_methods() -> Option<&'static Methods> {
        static RES: MethodsStatic = MethodsStatic::new();
        RES.methods(env_methods)
    }
}

starlark_simple_value!(Env);

#[starlark_module]
pub(crate) fn env_methods(registry: &mut MethodsBuilder) {
    /// Returns the version of the Aspect CLI.
    fn aspect_cli_version<'v>(
        #[allow(unused)] this: values::Value<'v>,
        eval: &mut Evaluator<'v, '_, '_>,
    ) -> anyhow::Result<values::StringValue<'v>> {
        let store = AxlStore::from_eval(eval)?;
        Ok(eval.heap().alloc_str(&store.cli_version))
    }

    /// Fetches the environment variable key from the current process.
    fn var<'v>(
        #[allow(unused)] this: values::Value<'v>,
        #[starlark(require = pos)] key: values::StringValue<'v>,
        heap: &'v Heap,
    ) -> anyhow::Result<NoneOr<values::StringValue<'v>>> {
        let val = std::env::var(key.as_str())
            .map(|val| heap.alloc_str(val.as_str()))
            .ok();
        Ok(NoneOr::from_option(val))
    }

    /// Returns an iterator of (variable, value) pairs of strings, for all the
    /// environment variables of the current process.
    ///
    /// The returned iterator contains a snapshot of the process's environment
    /// variables at the time of this invocation. Modifications to environment
    /// variables afterwards will not be reflected in the returned iterator.
    fn vars<'v>(
        #[allow(unused)] this: values::Value<'v>,
        heap: &'v Heap,
    ) -> anyhow::Result<
        ValueOfUnchecked<
            'v,
            UnpackList<ValueOfUnchecked<'v, UnpackTuple<values::StringValue<'v>>>>,
        >,
    > {
        Ok(heap
            .alloc_typed_unchecked(AllocList(std::env::vars().map(|(k, v)| {
                let val = [heap.alloc_str(k.as_str()), heap.alloc_str(v.as_str())];
                heap.alloc_typed_unchecked(AllocTuple(val))
                    .cast::<UnpackTuple<values::StringValue<'v>>>()
            })))
            .cast())
    }

    /// Returns the path of a temporary directory.
    ///
    /// The temporary directory may be shared among users, or between processes
    /// with different privileges; thus, the creation of any files or directories
    /// in the temporary directory must use a secure method to create a uniquely
    /// named file. Creating a file or directory with a fixed or predictable name
    /// may result in "insecure temporary file" security vulnerabilities. Consider
    /// using a crate that securely creates temporary files or directories.
    ///
    /// Note that the returned value may be a symbolic link, not a directory.
    ///
    ///
    /// **Platform**-specific behavior
    ///
    /// On Unix, returns the value of the `TMPDIR` environment variable if it is
    /// set, otherwise the value is OS-specific:
    /// - On Darwin-based OSes (macOS, iOS, etc) it returns the directory provided
    ///   by `confstr(_CS_DARWIN_USER_TEMP_DIR, ...)`, as recommended by [Apple's
    ///   security guidelines][appledoc].
    /// - On all other unix-based OSes, it returns `/tmp`.
    ///
    /// On Windows, the behavior is equivalent to that of [`GetTempPath2`][GetTempPath2] /
    /// [`GetTempPath`][GetTempPath], which this function uses internally.
    ///
    /// [GetTempPath2]: https://docs.microsoft.com/en-us/windows/win32/api/fileapi/nf-fileapi-gettemppath2a
    /// [GetTempPath]: https://docs.microsoft.com/en-us/windows/win32/api/fileapi/nf-fileapi-gettemppatha
    /// [appledoc]: https://developer.apple.com/library/archive/documentation/Security/Conceptual/SecureCodingGuide/Articles/RaceConditions.html#//apple_ref/doc/uid/TP40002585-SW10
    fn temp_dir<'v>(
        #[allow(unused)] this: values::Value<'v>,
        heap: &'v Heap,
    ) -> anyhow::Result<values::StringValue<'v>> {
        Ok(heap.alloc_str(
            std::env::temp_dir()
                // to_str() returns None() if string is not UTF-8 (https://doc.rust-lang.org/std/path/struct.Path.html#method.to_str)
                .to_str()
                .ok_or(anyhow::anyhow!("temp directory is non utf-8"))?,
        ))
    }

    /// Returns the path of the current user's home directory if known.
    ///
    /// This may return `None` if getting the directory fails or if the platform does not have user home directories.
    ///
    /// For storing user data and configuration it is often preferable to use more specific directories.
    /// For example, [XDG Base Directories] on Unix or the `LOCALAPPDATA` and `APPDATA` environment variables on Windows.
    ///
    /// [XDG Base Directories]: https://specifications.freedesktop.org/basedir-spec/latest/
    ///
    /// **Unix**
    ///
    /// - Returns the value of the 'HOME' environment variable if it is set
    ///   (including to an empty string).
    /// - Otherwise, it tries to determine the home directory by invoking the `getpwuid_r` function
    ///   using the UID of the current user. An empty home directory field returned from the
    ///   `getpwuid_r` function is considered to be a valid value.
    /// - Returns `None` if the current user has no entry in the /etc/passwd file.
    ///
    /// **Windows**
    ///
    /// - Returns the value of the 'USERPROFILE' environment variable if it is set, and is not an empty string.
    /// - Otherwise, [`GetUserProfileDirectory`][msdn] is used to return the path. This may change in the future.
    ///
    /// [msdn]: https://docs.microsoft.com/en-us/windows/win32/api/userenv/nf-userenv-getuserprofiledirectorya
    ///
    /// In UWP (Universal Windows Platform) targets this function is unimplemented and always returns `None`.
    fn home_dir<'v>(
        #[allow(unused)] this: values::Value<'v>,
        heap: &'v Heap,
    ) -> anyhow::Result<NoneOr<values::StringValue<'v>>> {
        Ok(match std::env::home_dir() {
            Some(path) => NoneOr::Other(
                heap.alloc_str(
                    path
                        // to_str() returns None() if string is not UTF-8 (https://doc.rust-lang.org/std/path/struct.Path.html#method.to_str)
                        .to_str()
                        .ok_or(anyhow::anyhow!("home directory is non utf-8"))?,
                ),
            ),
            None => NoneOr::None,
        })
    }

    /// Returns the current working directory as a path.
    ///
    /// **Platform**-specific behavior
    ///
    /// This function currently corresponds to the `getcwd` function on Unix
    /// and the `GetCurrentDirectoryW` function on Windows.
    ///
    ///
    /// **Errors**
    ///
    /// Fails if the current working directory value is invalid.
    /// Possible cases:
    ///
    /// * Current directory does not exist.
    /// * There are insufficient permissions to access the current directory.
    ///
    fn current_dir<'v>(
        #[allow(unused)] this: values::Value<'v>,
        heap: &'v Heap,
    ) -> anyhow::Result<values::StringValue<'v>> {
        Ok(heap.alloc_str(
            std::env::current_dir()?
                .to_str()
                // to_str() returns None() if string is not UTF-8 (https://doc.rust-lang.org/std/path/struct.Path.html#method.to_str)
                .ok_or(anyhow::anyhow!("current directory is non utf-8"))?,
        ))
    }

    fn current_exe<'v>(
        #[allow(unused)] this: values::Value<'v>,
        heap: &'v Heap,
    ) -> anyhow::Result<values::StringValue<'v>> {
        Ok(heap.alloc_str(
            std::env::current_exe()?
                .to_str()
                // to_str() returns None() if string is not UTF-8 (https://doc.rust-lang.org/std/path/struct.Path.html#method.to_str)
                .ok_or(anyhow::anyhow!("current executable is non utf-8"))?,
        ))
    }

    /// Returns the project root directory.
    ///
    /// This project root directory is found starting at current working directory and searching upwards
    /// through its ancestors for repository boundary marker files (such as `MODULE.aspect`, `MODULE.bazel`,
    /// `MODULE.bazel.lock`, `REPO.bazel`, `WORKSPACE`, or `WORKSPACE.bazel`). The first ancestor directory
    /// containing any of these files is considered the project root. If no such directory is found, the
    /// current directory is used as the project root.
    fn root_dir<'v>(
        #[allow(unused)] this: values::Value<'v>,
        eval: &mut Evaluator<'v, '_, '_>,
    ) -> anyhow::Result<values::StringValue<'v>> {
        let store = AxlStore::from_eval(eval)?;
        Ok(eval.heap().alloc_str(
            &store
                .root_dir
                .to_str()
                // to_str() returns None() if string is not UTF-8 (https://doc.rust-lang.org/std/path/struct.Path.html#method.to_str)
                .ok_or(anyhow::anyhow!("root dir is non utf-8"))?,
        ))
    }

    /// Returns the operating system name.
    ///
    /// Returns a string describing the operating system in use, such as
    /// "linux", "macos", "windows", etc.
    fn os<'v>(
        #[allow(unused)] this: values::Value<'v>,
        _heap: &'v Heap,
    ) -> anyhow::Result<&'v str> {
        Ok(std::env::consts::OS)
    }

    /// Returns the CPU architecture.
    ///
    /// Returns a string describing the CPU architecture, such as
    /// "x86_64", "aarch64", etc.
    fn arch<'v>(
        #[allow(unused)] this: values::Value<'v>,
        _heap: &'v Heap,
    ) -> anyhow::Result<&'v str> {
        Ok(std::env::consts::ARCH)
    }
}
