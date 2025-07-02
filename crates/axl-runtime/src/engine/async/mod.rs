use starlark::{
    environment::GlobalsBuilder, starlark_module,
    values::starlark_value_as_type::StarlarkValueAsType,
};

pub mod future;
mod future_stream;
pub mod util;

pub mod rt {
    use anyhow::Ok;
    use starlark::{eval::Evaluator, values::ProvidesStaticType};
    use std::ops::Deref;
    use tokio::runtime::Handle;

    #[derive(Debug, ProvidesStaticType, Clone)]
    pub struct AsyncRuntime(pub Handle);

    impl AsyncRuntime {
        pub fn new() -> Self {
            Self(Handle::current())
        }

        pub fn from_eval<'v>(eval: &mut Evaluator<'v, '_, '_>) -> anyhow::Result<AsyncRuntime> {
            let extra = eval
                .extra
                .ok_or(anyhow::anyhow!("Async runtime is not set"))?;

            let rt = extra.downcast_ref::<AsyncRuntime>().ok_or(anyhow::anyhow!(
                "failed to cast extra value to async runtime"
            ))?;

            Ok(AsyncRuntime(rt.0.clone()))
        }
    }

    impl Deref for AsyncRuntime {
        type Target = Handle;

        fn deref(&self) -> &Self::Target {
            &self.0
        }
    }
}

#[starlark_module]
fn register_type_toplevels(builder: &mut GlobalsBuilder) {
    const future: StarlarkValueAsType<future::StarlarkFuture> = StarlarkValueAsType::new();
}

pub fn register_toplevels(builder: &mut GlobalsBuilder) {
    util::register_toplevels(builder);
    register_type_toplevels(builder)
}
