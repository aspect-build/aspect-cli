use std::path::PathBuf;

use starlark::{eval::Evaluator, values::ProvidesStaticType};

use super::r#async::rt::AsyncRuntime;

/// A store object which we pass to the Starlark interpreter which allows us
/// to store shared data (runtime, tools, cache, ...) around the Starlark evaluation.
#[derive(Debug, ProvidesStaticType, Clone)]
pub struct AxlStore {
    pub aspect_cli_version: String,
    pub root_dir: PathBuf,
    pub rt: AsyncRuntime,
}

impl AxlStore {
    pub fn new(aspect_cli_version: &str, root_dir: &PathBuf) -> Self {
        Self {
            aspect_cli_version: aspect_cli_version.to_string(),
            root_dir: root_dir.clone(),
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
            aspect_cli_version: value.aspect_cli_version.clone(),
            root_dir: value.root_dir.clone(),
            rt: value.rt.clone(),
        })
    }
}
