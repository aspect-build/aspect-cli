use starlark::{
    environment::GlobalsBuilder, starlark_module,
    values::starlark_value_as_type::StarlarkValueAsType,
};

pub mod future;
mod future_stream;
pub mod util;

pub mod rt {
    use starlark::values::ProvidesStaticType;
    use std::ops::Deref;
    use tokio::runtime::Handle;

    #[derive(Debug, ProvidesStaticType, Clone)]
    pub struct AsyncRuntime(pub Handle);

    impl AsyncRuntime {
        pub fn new() -> Self {
            Self(Handle::current())
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
fn register_types(globals: &mut GlobalsBuilder) {
    const Future: StarlarkValueAsType<future::StarlarkFuture> = StarlarkValueAsType::new();
}

pub fn register_globals(globals: &mut GlobalsBuilder) {
    register_types(globals);

    util::register_globals(globals);
}
