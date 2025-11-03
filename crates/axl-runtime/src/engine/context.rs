use starlark::{eval::Evaluator, values::ProvidesStaticType};

use super::r#async::rt::AsyncRuntime;

/// A context object which we pass to the Starlark interpreter which allows us
/// to track state (tools, cache, ...) around the Starlark evaluation.
#[derive(Debug, ProvidesStaticType, Clone)]
pub struct AxlContext {
    pub rt: AsyncRuntime,
}

impl AxlContext {
    pub fn new() -> Self {
        Self {
            rt: AsyncRuntime::new(),
        }
    }

    pub fn from_eval<'v>(eval: &mut Evaluator<'v, '_, '_>) -> anyhow::Result<AxlContext> {
        let value = eval
            .extra
            .ok_or(anyhow::anyhow!("failed to get axl context"))?
            .downcast_ref::<AxlContext>()
            .ok_or(anyhow::anyhow!("failed to cast axl context"))?;
        Ok(AxlContext {
            rt: value.rt.clone(),
        })
    }
}