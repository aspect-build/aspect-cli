use allocative::Allocative;
use derive_more::Display;

use starlark::environment::Methods;
use starlark::environment::MethodsBuilder;
use starlark::environment::MethodsStatic;

use starlark::starlark_module;
use starlark::starlark_simple_value;
use starlark::values;
use starlark::values::starlark_value;
use starlark::values::NoSerialize;
use starlark::values::ProvidesStaticType;
use starlark::values::Trace;

use super::bazel::Bazel;
use super::http::Http;
use super::std::Std;
use super::template;
use super::wasm::Wasm;

#[derive(Debug, Clone, ProvidesStaticType, Display, Trace, NoSerialize, Allocative)]
#[display("<ConfigContext>")]
pub struct ConfigContext<'v> {
    _phantom: std::marker::PhantomData<&'v ()>,
}

impl<'v> ConfigContext<'v> {
    pub fn new() -> Self {
        Self {
            _phantom: std::marker::PhantomData,
        }
    }
}

#[starlark_value(type = "ConfigContext")]
impl<'v> values::StarlarkValue<'v> for ConfigContext<'v> {
    fn get_methods() -> Option<&'static Methods> {
        static RES: MethodsStatic = MethodsStatic::new();
        RES.methods(config_context_methods)
    }
}

impl<'v> values::AllocValue<'v> for ConfigContext<'v> {
    fn alloc_value(self, heap: &'v values::Heap) -> values::Value<'v> {
        heap.alloc_complex(self)
    }
}

impl<'v> values::Freeze for ConfigContext<'v> {
    type Frozen = FrozenConfigContext;
    fn freeze(self, _freezer: &values::Freezer) -> values::FreezeResult<Self::Frozen> {
        panic!("not implemented")
    }
}

#[derive(Debug, Display, ProvidesStaticType, NoSerialize, Allocative)]
#[display("<ConfigContext>")]
pub struct FrozenConfigContext {}

starlark_simple_value!(FrozenConfigContext);

#[starlark_value(type = "ConfigContext")]
impl<'v> values::StarlarkValue<'v> for FrozenConfigContext {
    type Canonical = ConfigContext<'v>;
}

#[starlark_module]
pub(crate) fn config_context_methods(registry: &mut MethodsBuilder) {
    /// Standard library is the foundation of powerful AXL tasks.
    #[starlark(attribute)]
    fn std<'v>(#[allow(unused)] this: values::Value<'v>) -> starlark::Result<Std> {
        Ok(Std {})
    }

    /// Expand template files.
    #[starlark(attribute)]
    fn template<'v>(
        #[allow(unused)] this: values::Value<'v>,
    ) -> starlark::Result<template::Template> {
        Ok(template::Template::new())
    }

    /// EXPERIMENTAL! Run wasm programs within tasks.
    #[starlark(attribute)]
    fn wasm<'v>(#[allow(unused)] this: values::Value<'v>) -> starlark::Result<Wasm> {
        Ok(Wasm::new())
    }

    /// Access to Bazel functionality.
    #[starlark(attribute)]
    fn bazel<'v>(#[allow(unused)] this: values::Value<'v>) -> starlark::Result<Bazel> {
        Ok(Bazel {})
    }

    /// The `http` attribute provides a programmatic interface for making HTTP requests.
    /// It is used to fetch data from remote servers and can be used in conjunction with
    /// other aspects to perform complex data processing tasks.
    ///
    /// **Example**
    ///
    /// ```starlark
    /// **Fetch** data from a remote server
    /// data = ctx.http().get("https://example.com/data.json").block()
    /// ```
    fn http<'v>(#[allow(unused)] this: values::Value<'v>) -> starlark::Result<Http> {
        Ok(Http::new())
    }
}
