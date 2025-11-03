use starlark::{eval::Evaluator, values::ProvidesStaticType};

use super::r#async::rt::AsyncRuntime;

/// A store object which we pass to the Starlark interpreter which allows us
/// to store shared data (runtime, tools, cache, ...) around the Starlark evaluation.
#[derive(Debug, ProvidesStaticType, Clone)]
pub struct AxlStore {
    pub rt: AsyncRuntime,
}

impl AxlStore {
    pub fn new() -> Self {
        Self {
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
            rt: value.rt.clone(),
        })
    }
}