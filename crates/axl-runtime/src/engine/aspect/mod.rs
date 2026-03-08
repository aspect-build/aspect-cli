use allocative::Allocative;
use derive_more::Display;

use starlark::environment::Methods;
use starlark::environment::MethodsBuilder;
use starlark::environment::MethodsStatic;
use starlark::starlark_module;
use starlark::starlark_simple_value;
use starlark::values;
use starlark::values::NoSerialize;
use starlark::values::ProvidesStaticType;
use starlark::values::starlark_value;

use starlark::{environment::GlobalsBuilder, values::starlark_value_as_type::StarlarkValueAsType};

pub mod auth;

#[derive(Debug, Display, ProvidesStaticType, NoSerialize, Allocative)]
#[display("<aspect.Aspect>")]
pub struct Aspect {}

starlark_simple_value!(Aspect);

#[starlark_value(type = "aspect.Aspect")]
impl<'v> values::StarlarkValue<'v> for Aspect {
    fn get_methods() -> Option<&'static Methods> {
        static RES: MethodsStatic = MethodsStatic::new();
        RES.methods(aspect_methods)
    }
}

#[starlark_module]
pub(crate) fn aspect_methods(registry: &mut MethodsBuilder) {
    #[starlark(attribute)]
    fn auth<'v>(#[allow(unused)] this: values::Value<'v>) -> anyhow::Result<auth::Auth> {
        Ok(auth::Auth {})
    }
}

#[starlark_module]
fn register_types(globals: &mut GlobalsBuilder) {
    const Aspect: StarlarkValueAsType<Aspect> = StarlarkValueAsType::new();
}

pub fn register_globals(globals: &mut GlobalsBuilder) {
    register_types(globals);
    auth::register_globals(globals);
}
