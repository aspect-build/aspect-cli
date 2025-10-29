use allocative::Allocative;
use derive_more::Display;

use starlark::environment::GlobalsBuilder;
use starlark::environment::Methods;
use starlark::environment::MethodsBuilder;
use starlark::environment::MethodsStatic;

use starlark::starlark_module;
use starlark::starlark_simple_value;
use starlark::values;
use starlark::values::starlark_value;
use starlark::values::starlark_value_as_type::StarlarkValueAsType;
use starlark::values::NoSerialize;
use starlark::values::ProvidesStaticType;
use starlark::values::Trace;
use starlark::values::ValueLike;

use super::http::Http;
use super::std::Std;
use super::task_args::TaskArgs;
use super::template;

#[derive(Debug, Clone, ProvidesStaticType, Display, Trace, NoSerialize, Allocative)]
#[display("<config_context>")]
pub struct ConfigContext<'v> {
    pub args: TaskArgs<'v>,
}

impl<'v> ConfigContext<'v> {
    pub fn new(args: TaskArgs<'v>) -> Self {
        Self { args }
    }
}

#[starlark_value(type = "config_context")]
impl<'v> values::StarlarkValue<'v> for ConfigContext<'v> {
    fn get_methods() -> Option<&'static Methods> {
        static RES: MethodsStatic = MethodsStatic::new();
        RES.methods(config_context_methods)
    }
}

#[starlark_module]
pub(crate) fn config_context_methods(registry: &mut MethodsBuilder) {
    /// Standard library is the foundation of powerful AXL tasks.
    #[starlark(attribute)]
    fn std<'v>(#[allow(unused)] this: values::Value<'v>) -> starlark::Result<Std> {
        Ok(Std {})
    }

    /// Access to arguments provided by the caller.
    #[starlark(attribute)]
    fn args<'v>(#[allow(unused)] this: values::Value<'v>) -> starlark::Result<TaskArgs<'v>> {
        let ctx = this.downcast_ref_err::<ConfigContext>()?;
        // TODO: don't do this.
        Ok(ctx.args.clone())
    }

    /// Expand template files.
    #[starlark(attribute)]
    fn template<'v>(
        #[allow(unused)] this: values::Value<'v>,
    ) -> starlark::Result<template::Template> {
        Ok(template::Template::new())
    }

    /// The `http` attribute provides a programmatic interface for making HTTP requests.
    /// It is used to fetch data from remote servers and can be used in conjunction with
    /// other aspects to perform complex data processing tasks.
    ///
    /// # Example
    ///
    /// ```starlark
    /// # Fetch data from a remote server
    /// data = ctx.http().get("https://example.com/data.json").block()
    /// ```
    fn http<'v>(#[allow(unused)] this: values::Value<'v>) -> starlark::Result<Http> {
        Ok(Http::new())
    }

    #[starlark(attribute)]
    fn os<'v>(#[allow(unused)] this: values::Value<'v>) -> starlark::Result<Std> {
        Ok(Os {})
    }
}

#[starlark_module]
pub fn register_toplevels(_: &mut GlobalsBuilder) {
    const config_context: StarlarkValueAsType<ConfigContext> = StarlarkValueAsType::new();
}
