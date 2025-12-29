use std::path::PathBuf;

use starlark::{eval::Evaluator, values::ProvidesStaticType};

use super::r#async::rt::AsyncRuntime;

/// A store object which we pass to the Starlark interpreter which allows us
/// to store shared data (runtime, tools, cache, ...) around the Starlark evaluation.
#[derive(Debug, ProvidesStaticType, Clone)]
pub struct AxlStore {
    pub(crate) cli_version: String,
    pub(crate) root_dir: PathBuf,
    pub(crate) script_path: PathBuf,
    pub(crate) rt: AsyncRuntime,
}

impl AxlStore {
    pub fn new(cli_version: String, root_dir: PathBuf, script_path: PathBuf) -> Self {
        Self {
            cli_version,
            root_dir: root_dir,
            script_path: script_path,
            rt: AsyncRuntime::new(),
        }
    }

    pub fn from_eval<'v>(eval: &mut Evaluator<'v, '_, '_>) -> anyhow::Result<AxlStore> {
        let value = eval
            .extra
            .as_ref()
            .ok_or_else(|| anyhow::anyhow!("failed to get axl store (extra is None)"))?;

        // Try both &AxlStore and AxlStore casts as you may get one or the other depending on how
        // Rust decides to compile a `eval.extra = Some(&store)`
        if let Some(store_ref) = value.downcast_ref::<&AxlStore>() {
            return Ok(AxlStore {
                cli_version: store_ref.cli_version.clone(),
                root_dir: store_ref.root_dir.clone(),
                script_path: store_ref.script_path.clone(),
                rt: store_ref.rt.clone(),
            });
        }

        if let Some(store_owned) = value.downcast_ref::<AxlStore>() {
            return Ok(AxlStore {
                cli_version: store_owned.cli_version.clone(),
                root_dir: store_owned.root_dir.clone(),
                script_path: store_owned.script_path.clone(),
                rt: store_owned.rt.clone(),
            });
        }

        Err(anyhow::anyhow!(
            "failed to cast axl store: unexpected type (not AxlStore nor &AxlStore). Actual type: {}",
            std::any::type_name_of_val(value)
        ))
    }
}
