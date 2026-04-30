use std::path::PathBuf;

use starlark::{eval::Evaluator, values::ProvidesStaticType};

use super::r#async::rt::AsyncRuntime;

/// Process-wide environment passed to every Starlark evaluator via `eval.extra`.
///
/// `script_path` is intentionally absent — the file currently being evaluated
/// is recovered from the evaluator's call stack by [`Env::current_script_path`].
#[derive(Debug, ProvidesStaticType, Clone)]
pub struct Env {
    pub cli_version: String,
    pub root_dir: PathBuf,
    pub rt: AsyncRuntime,
}

impl Env {
    pub fn new(cli_version: String, root_dir: PathBuf) -> Self {
        Self {
            cli_version,
            root_dir,
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
