use std::path::{Path, PathBuf};
use std::sync::OnceLock;

use starlark::{eval::Evaluator, values::ProvidesStaticType};

use super::r#async::rt::AsyncRuntime;

/// The Aspect project root this process resolved, recorded by [`Env::new`] for
/// the code that needs it without an evaluator to read [`Env`] from.
static ASPECT_ROOT_DIR: OnceLock<PathBuf> = OnceLock::new();

/// The Aspect project root this process resolved at startup, or `None` in a
/// process that never built an [`Env`] — the `aspect get` credential helper,
/// which Bazel spawns as a bare subprocess and which deliberately skips
/// workspace discovery. Callers that still want an answer there resolve one
/// themselves with [`crate::project_root`].
pub fn resolved_aspect_root() -> Option<&'static Path> {
    ASPECT_ROOT_DIR.get().map(PathBuf::as_path)
}

/// Process-wide environment passed to every Starlark evaluator via `eval.extra`.
///
/// `script_path` is intentionally absent — the file currently being evaluated
/// is recovered from the evaluator's call stack by [`Env::current_script_path`].
#[derive(Debug, ProvidesStaticType, Clone)]
pub struct Env {
    pub cli_version: String,
    /// Aspect project root — anchor for axl / config loading.
    pub aspect_root_dir: PathBuf,
    /// Bazel workspace root — anchor for bazelrc discovery,
    /// `bazel info workspace`, and BES output paths. Distinct from
    /// `aspect_root_dir` when a Bazel sub-workspace sits under an
    /// Aspect root.
    pub bazel_root_dir: PathBuf,
    /// Git repository root — directory containing `.git`. `None` when
    /// not inside a git repository.
    pub git_root_dir: Option<PathBuf>,
    pub rt: AsyncRuntime,
}

impl Env {
    pub fn new(
        cli_version: String,
        aspect_root_dir: PathBuf,
        bazel_root_dir: PathBuf,
        git_root_dir: Option<PathBuf>,
    ) -> Self {
        // First one wins: a process resolves the root once, and a later `Env`
        // (the test harness builds several) does not move it.
        let _ = ASPECT_ROOT_DIR.set(aspect_root_dir.clone());
        Self {
            cli_version,
            aspect_root_dir,
            bazel_root_dir,
            git_root_dir,
            rt: AsyncRuntime::new(),
        }
    }

    pub fn from_eval<'v, 'a>(eval: &'a Evaluator<'v, '_, '_>) -> anyhow::Result<&'a Env> {
        let value = eval
            .extra
            .as_ref()
            .ok_or_else(|| anyhow::anyhow!("failed to get env (extra is None)"))?;
        value.downcast_ref::<Env>().ok_or_else(|| {
            anyhow::anyhow!(
                "failed to cast env: unexpected type. Actual type: {}",
                std::any::type_name_of_val(value)
            )
        })
    }

    /// Absolute path of the `.axl` file currently being evaluated.
    ///
    /// Reads the topmost call-stack frame, whose filename is whatever was
    /// passed to `AstModule::parse` — `AxlLoader` always passes the absolute
    /// path. Returns an error if there is no Starlark frame on the stack
    /// (i.e. called from native-only context).
    pub fn current_script_path(eval: &Evaluator) -> anyhow::Result<PathBuf> {
        let span = eval.call_stack_top_location().ok_or_else(|| {
            anyhow::anyhow!("no Starlark frame on the call stack — cannot resolve script path")
        })?;
        Ok(PathBuf::from(span.filename()))
    }
}
