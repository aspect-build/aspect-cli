use std::collections::BTreeMap;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};

use starlark::{eval::Evaluator, values::ProvidesStaticType};

use super::r#async::rt::AsyncRuntime;

/// In-memory environment-variable overlay shared between a test's harness
/// (`t.env`) and the `std.env` backend reachable through `t.std.env` /
/// `t.ctx.std.env`.
///
/// Shared via a handle so a mutation through one view (e.g. `t.env.set(...)`)
/// is observed through the others (`t.ctx.std.env.var(...)`) — they all hold
/// clones of the same handle onto one map. A `BTreeMap` keeps `vars()`
/// iteration deterministic for snapshot-style assertions.
///
/// `Arc<Mutex<…>>` (not `Rc<RefCell<…>>`) so the handle is `Send + Sync`: the
/// `std.Env` / `Std` Starlark values that now *carry* it must satisfy the
/// `Send + Sync` bound that frozen Starlark values require. Each test's
/// overlay is only ever touched on that test's own worker thread, so the
/// `Mutex` is never actually contended — it is correctness insurance for the
/// parallel runner, not a hot path.
pub type TestEnvMap = Arc<Mutex<BTreeMap<String, String>>>;

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
