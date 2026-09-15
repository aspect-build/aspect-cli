use allocative::Allocative;
use derive_more::Display;
use starlark::environment::{Methods, MethodsBuilder, MethodsStatic};
use starlark::eval::Evaluator;
use starlark::values::list::{AllocList, UnpackList};
use starlark::values::none::NoneOr;
use starlark::values::tuple::{AllocTuple, UnpackTuple};
use starlark::values::{Heap, NoSerialize, ProvidesStaticType, ValueOfUnchecked};
use starlark::values::{StarlarkValue, starlark_value};
use starlark::{starlark_module, starlark_simple_value, values};

use crate::engine::store::Env as RuntimeEnv;

#[derive(Clone, Debug, ProvidesStaticType, NoSerialize, Allocative, Display)]
#[display("<std.Env>")]
pub struct Env {}

impl Env {
    pub fn new() -> Self {
        Self {}
    }
}

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
        let env = RuntimeEnv::from_eval(eval)?;
        Ok(eval.heap().alloc_str(&env.cli_version))
    }

    /// Fetches the environment variable key from the current process.
    fn var<'v>(
        #[allow(unused)] this: values::Value<'v>,
        #[starlark(require = pos)] key: values::StringValue<'v>,
        heap: Heap<'v>,
    ) -> anyhow::Result<NoneOr<values::StringValue<'v>>> {
        let val = std::env::var(key.as_str())
            .map(|val| heap.alloc_str(val.as_str()))
            .ok();
        Ok(NoneOr::from_option(val))
    }

    /// Sets an environment variable for the current process.
    ///
    /// This affects all subsequent `var()` calls and any child processes spawned
    /// after this call. Use with care — environment variables are global process
    /// state.
    fn set_var<'v>(
        #[allow(unused)] this: values::Value<'v>,
        #[starlark(require = pos)] key: values::StringValue<'v>,
        #[starlark(require = pos)] value: values::StringValue<'v>,
    ) -> anyhow::Result<values::none::NoneType> {
        // SAFETY: AXL evaluation is single-threaded; no concurrent env reads.
        #[allow(deprecated)]
        unsafe {
            std::env::set_var(key.as_str(), value.as_str());
        }
        Ok(values::none::NoneType)
    }

    /// Removes an environment variable from the current process.
    ///
    /// Has no effect if the variable is not set. Subsequent `var()` calls will
    /// return `None` for the removed variable.
    fn remove_var<'v>(
        #[allow(unused)] this: values::Value<'v>,
        #[starlark(require = pos)] key: values::StringValue<'v>,
    ) -> anyhow::Result<values::none::NoneType> {
        // SAFETY: AXL evaluation is single-threaded; no concurrent env reads.
        #[allow(deprecated)]
        unsafe {
            std::env::remove_var(key.as_str());
        }
        Ok(values::none::NoneType)
    }

    /// Returns an iterator of (variable, value) pairs of strings, for all the
    /// environment variables of the current process.
    ///
    /// The returned iterator contains a snapshot of the process's environment
    /// variables at the time of this invocation. Modifications to environment
    /// variables afterwards will not be reflected in the returned iterator.
    fn vars<'v>(
        #[allow(unused)] this: values::Value<'v>,
        heap: Heap<'v>,
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
        heap: Heap<'v>,
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
        heap: Heap<'v>,
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
        heap: Heap<'v>,
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
        heap: Heap<'v>,
    ) -> anyhow::Result<values::StringValue<'v>> {
        Ok(heap.alloc_str(
            std::env::current_exe()?
                .to_str()
                // to_str() returns None() if string is not UTF-8 (https://doc.rust-lang.org/std/path/struct.Path.html#method.to_str)
                .ok_or(anyhow::anyhow!("current executable is non utf-8"))?,
        ))
    }

    /// Returns the Aspect project root directory — the anchor for axl /
    /// config loading.
    ///
    /// Found by walking upwards from the current working directory looking
    /// for an `.aspect/version.axl` or `MODULE.aspect` marker. Falls back
    /// to the deepest Bazel workspace marker (`MODULE.bazel`, `WORKSPACE`,
    /// etc.) so a pure-Bazel monorepo still resolves to a sane project
    /// anchor, and finally to the current working directory if no marker
    /// exists anywhere.
    ///
    /// Distinct from the Bazel workspace root — bazelrc discovery and
    /// `bazel info workspace` use the deepest Bazel marker, which can
    /// differ when a Bazel sub-workspace sits under an Aspect root.
    fn aspect_root_dir<'v>(
        #[allow(unused)] this: values::Value<'v>,
        eval: &mut Evaluator<'v, '_, '_>,
    ) -> anyhow::Result<values::StringValue<'v>> {
        let env = RuntimeEnv::from_eval(eval)?;
        let s = env
            .aspect_root_dir
            .to_str()
            .ok_or_else(|| anyhow::anyhow!("aspect root dir is non utf-8"))?;
        Ok(eval.heap().alloc_str(s))
    }

    /// Returns the git repository root — the directory containing `.git` —
    /// or `None` when not inside a git repository.
    fn git_root_dir<'v>(
        #[allow(unused)] this: values::Value<'v>,
        eval: &mut Evaluator<'v, '_, '_>,
    ) -> anyhow::Result<NoneOr<values::StringValue<'v>>> {
        let env = RuntimeEnv::from_eval(eval)?;
        Ok(match &env.git_root_dir {
            Some(path) => {
                let s = path
                    .to_str()
                    .ok_or_else(|| anyhow::anyhow!("git root dir is non utf-8"))?;
                NoneOr::Other(eval.heap().alloc_str(s))
            }
            None => NoneOr::None,
        })
    }

    /// Returns the Bazel workspace root directory — the anchor for bazelrc
    /// discovery, `bazel info workspace`, and BES output paths.
    ///
    /// Found by walking upwards from the current working directory looking
    /// for a `MODULE.bazel` / `WORKSPACE` marker. Falls back to the deepest
    /// `.aspect/version.axl` or `MODULE.aspect` marker so a pure-Aspect
    /// workspace still resolves, and finally to the current working
    /// directory if no marker exists anywhere.
    ///
    /// Distinct from the Aspect project root — axl / config loading uses
    /// the deepest `.aspect/` marker, which can differ when a Bazel
    /// sub-workspace sits under an Aspect root.
    fn bazel_root_dir<'v>(
        #[allow(unused)] this: values::Value<'v>,
        eval: &mut Evaluator<'v, '_, '_>,
    ) -> anyhow::Result<values::StringValue<'v>> {
        let env = RuntimeEnv::from_eval(eval)?;
        let s = env
            .bazel_root_dir
            .to_str()
            .ok_or_else(|| anyhow::anyhow!("bazel root dir is non utf-8"))?;
        Ok(eval.heap().alloc_str(s))
    }

    /// Returns the operating system name.
    ///
    /// Returns a string describing the operating system in use, such as
    /// "linux", "macos", "windows", etc.
    fn os<'v>(
        #[allow(unused)] this: values::Value<'v>,
        _heap: Heap<'v>,
    ) -> anyhow::Result<&'v str> {
        Ok(std::env::consts::OS)
    }

    /// Returns the CPU architecture.
    ///
    /// Returns a string describing the CPU architecture, such as
    /// "x86_64", "aarch64", etc.
    fn arch<'v>(
        #[allow(unused)] this: values::Value<'v>,
        _heap: Heap<'v>,
    ) -> anyhow::Result<&'v str> {
        Ok(std::env::consts::ARCH)
    }

    /// The absolute path a shell would run `name` as, or `None` when no `PATH`
    /// entry holds an executable of that name — `which` / `command -v`.
    ///
    /// A bare name is looked up along `PATH`; a name containing a path
    /// separator is checked as given. On Unix the file must carry an execute
    /// bit; on Windows the `PATHEXT` extensions are tried.
    ///
    /// **Examples**
    ///
    /// ```python
    /// helper = "aspect" if ctx.std.env.which("aspect") else ctx.std.env.current_exe()
    /// ```
    fn which<'v>(
        #[allow(unused)] this: values::Value<'v>,
        #[starlark(require = pos)] name: &str,
        heap: Heap<'v>,
    ) -> anyhow::Result<NoneOr<values::StringValue<'v>>> {
        Ok(
            match find_executable(name, std::env::var_os("PATH").as_deref()) {
                Some(path) => NoneOr::Other(
                    heap.alloc_str(
                        path.to_str()
                            .ok_or_else(|| anyhow::anyhow!("path of `{name}` is non utf-8"))?,
                    ),
                ),
                None => NoneOr::None,
            },
        )
    }
}

/// Resolve `name` the way a shell does against `path` (the `PATH` value): a
/// bare name against each entry in order, a name with a separator as given.
/// The first existing executable wins.
pub(crate) fn find_executable(
    name: &str,
    path: Option<&std::ffi::OsStr>,
) -> Option<std::path::PathBuf> {
    let candidate = std::path::Path::new(name);
    if candidate.components().count() > 1 {
        return executable_variants(candidate)
            .into_iter()
            .find(|p| is_executable(p));
    }
    std::env::split_paths(path?)
        .filter(|dir| !dir.as_os_str().is_empty())
        .flat_map(|dir| executable_variants(&dir.join(name)))
        .find(|p| is_executable(p))
}

#[cfg(unix)]
fn executable_variants(base: &std::path::Path) -> Vec<std::path::PathBuf> {
    vec![base.to_path_buf()]
}

#[cfg(windows)]
fn executable_variants(base: &std::path::Path) -> Vec<std::path::PathBuf> {
    let mut variants = vec![base.to_path_buf()];
    if let Some(exts) = std::env::var_os("PATHEXT") {
        for ext in std::env::split_paths(&exts) {
            let mut with_ext = base.as_os_str().to_owned();
            with_ext.push(ext.as_os_str());
            variants.push(std::path::PathBuf::from(with_ext));
        }
    }
    variants
}

#[cfg(unix)]
fn is_executable(path: &std::path::Path) -> bool {
    use std::os::unix::fs::PermissionsExt;
    path.metadata()
        .map(|m| m.is_file() && m.permissions().mode() & 0o111 != 0)
        .unwrap_or(false)
}

#[cfg(windows)]
fn is_executable(path: &std::path::Path) -> bool {
    path.is_file()
}

#[cfg(all(test, unix))]
mod tests {
    use super::find_executable;
    use std::os::unix::fs::PermissionsExt;

    fn file(dir: &std::path::Path, name: &str, mode: u32) -> std::path::PathBuf {
        let p = dir.join(name);
        std::fs::write(&p, "#!/bin/sh\n").unwrap();
        std::fs::set_permissions(&p, std::fs::Permissions::from_mode(mode)).unwrap();
        p
    }

    #[test]
    fn which_walks_path_in_order_and_requires_an_execute_bit() {
        let first = tempfile::tempdir().unwrap();
        let second = tempfile::tempdir().unwrap();
        file(first.path(), "aspect", 0o644);
        let runnable = file(second.path(), "aspect", 0o755);
        let path = std::env::join_paths([first.path(), second.path()]).unwrap();

        // The non-executable file in the first entry is skipped; the second wins.
        assert_eq!(
            find_executable("aspect", Some(&path)),
            Some(runnable.clone())
        );
        assert_eq!(find_executable("bazel", Some(&path)), None);
        assert_eq!(find_executable("aspect", None), None);

        // A name with a separator is checked as given, not along PATH.
        assert_eq!(
            find_executable(runnable.to_str().unwrap(), Some(&path)),
            Some(runnable)
        );
        assert_eq!(
            find_executable(first.path().join("aspect").to_str().unwrap(), Some(&path)),
            None
        );
    }
}
