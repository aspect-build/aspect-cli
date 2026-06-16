use std::cell::RefCell;
use std::collections::BTreeMap;
use std::path::PathBuf;
use std::rc::Rc;

use starlark::{eval::Evaluator, values::ProvidesStaticType};

use super::r#async::rt::AsyncRuntime;

/// In-memory environment-variable overlay shared between a test's harness
/// (`t.env`) and the `std.env` backend reachable through `ctx.std.env`.
///
/// Shared via `Rc<RefCell<…>>` so a mutation through one view (e.g.
/// `t.env.set(...)`) is observed through the other (`ctx.std.env.var(...)`)
/// — they are two handles onto the same map. A `BTreeMap` keeps `vars()`
/// iteration deterministic for snapshot-style assertions.
pub type TestEnvMap = Rc<RefCell<BTreeMap<String, String>>>;

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

    /// When `Some`, the `std.env` builtins read and write this in-memory map
    /// instead of the real process environment. Installed per-test by the
    /// test runner (see [`crate::engine::testing`]) so env mutations during a
    /// test are isolated and never leak into the process — the real type
    /// (`std.Env`) is unchanged; only the backend it consults differs.
    pub test_env: Option<TestEnvMap>,
}

impl Env {
    pub fn new(
        cli_version: String,
        aspect_root_dir: PathBuf,
        bazel_root_dir: PathBuf,
        git_root_dir: Option<PathBuf>,
    ) -> Self {
        Self {
            cli_version,
            aspect_root_dir,
            bazel_root_dir,
            git_root_dir,
            rt: AsyncRuntime::new(),
            test_env: None,
        }
    }

    /// Clone this env with an in-memory `test_env` overlay installed. Used by
    /// the test runner to give each test an isolated, process-free environment
    /// that `std.env` transparently reads through.
    pub fn with_test_env(&self, map: TestEnvMap) -> Self {
        let mut cloned = self.clone();
        cloned.test_env = Some(map);
        cloned
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
