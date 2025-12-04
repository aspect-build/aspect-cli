use std::path::PathBuf;

use starlark::{eval::Evaluator, values::ProvidesStaticType};

use super::r#async::rt::AsyncRuntime;

/// A store object which we pass to the Starlark interpreter which allows us
/// to store shared data (runtime, tools, cache, ...) around the Starlark evaluation.
#[derive(Debug, ProvidesStaticType, Clone)]
pub struct AxlStore {
    pub(crate) cli_version: String,
    pub(crate) root_dir: PathBuf,
    pub(crate) rt: AsyncRuntime,
}

impl AxlStore {
    pub fn new(cli_version: String, root_dir: PathBuf) -> Self {
        Self {
            cli_version,
            root_dir: root_dir,
            rt: AsyncRuntime::new(),
        }
    }

    pub fn from_eval<'v>(eval: &mut Evaluator<'v, '_, '_>) -> anyhow::Result<AxlStore> {
        let value = eval
            .extra
            .ok_or(anyhow::anyhow!("failed to get axl store"))?
            .downcast_ref::<AxlStore>()
            .ok_or(anyhow::anyhow!("failed to cast axl store"))?;
        Ok(AxlStore {
            cli_version: value.cli_version.clone(),
            root_dir: value.root_dir.clone(),
            rt: value.rt.clone(),
        })
    }
}
