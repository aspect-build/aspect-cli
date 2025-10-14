mod delivery;

use allocative::Allocative;
use derive_more::Display;

use starlark::environment::Methods;
use starlark::environment::MethodsBuilder;
use starlark::environment::MethodsStatic;

use starlark::starlark_simple_value;
use starlark::values;
use starlark::values::none::NoneOr;
use starlark::values::none::NoneType;
use starlark::values::starlark_value;
use starlark::values::NoSerialize;
use starlark::values::ProvidesStaticType;

use starlark::{environment::GlobalsBuilder, starlark_module};

#[derive(Debug, Display, ProvidesStaticType, NoSerialize, Allocative)]
#[display("<delivery>")]
pub struct DeliveryModule {}

impl DeliveryModule {
    pub fn new() -> Self {
        Self {}
    }
}

starlark_simple_value!(DeliveryModule);

#[starlark_value(type = "delivery")]
impl<'v> values::StarlarkValue<'v> for DeliveryModule {
    fn get_methods() -> Option<&'static Methods> {
        static RES: MethodsStatic = MethodsStatic::new();
        RES.methods(module_methods)
    }
}

#[starlark_module]
pub(crate) fn module_methods(registry: &mut MethodsBuilder) {
    fn available<'v>(#[allow(unused)] this: values::Value<'v>) -> starlark::Result<bool> {
        Ok(false)
    }
    fn commit<'v>(#[allow(unused)] this: values::Value<'v>) -> starlark::Result<NoneOr<String>> {
        let commit = std::env::var("ASPECT_WORKFLOWS_DELIVERY_COMMIT")
            .ok()
            // DELIVERY_COMMIT handled for backwards compat with 5.10 and older
            .or_else(|| std::env::var("DELIVERY_COMMIT").ok());
        Ok(NoneOr::from_option(commit))
    }
    fn changed_since<'v>(#[allow(unused)] this: values::Value<'v>) -> starlark::Result<NoneType> {
        Ok(NoneType)
    }

    fn deliver<'v>(
        #[allow(unused)] this: values::Value<'v>,
        #[allow(unused)]
        #[starlark(require = pos)]
        target: values::StringValue,
    ) -> starlark::Result<NoneType> {
        Ok(NoneType)
    }
}

#[starlark_module]
pub fn register_toplevels(builder: &mut GlobalsBuilder) {
    //     const std: StarlarkValueAsType<Std> = StarlarkValueAsType::new();
}
