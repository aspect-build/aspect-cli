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
use starlark::values::ValueLike;

use super::bazel::Bazel;
use super::delivery::DeliveryModule;
use super::http::Http;
use super::std::Std;
use super::task_args::FrozenTaskArgs;
use super::task_args::TaskArgs;
use super::template;
use super::wasm::Wasm;

#[derive(Debug, Clone, ProvidesStaticType, Display, Trace, NoSerialize, Allocative)]
#[display("<task_context>")]
pub struct TaskContext<'v> {
    pub args: TaskArgs<'v>,
}

impl<'v> TaskContext<'v> {
    pub fn new(args: TaskArgs<'v>) -> Self {
        Self { args }
    }
}

#[starlark_value(type = "task_context")]
impl<'v> values::StarlarkValue<'v> for TaskContext<'v> {
    fn get_methods() -> Option<&'static Methods> {
        static RES: MethodsStatic = MethodsStatic::new();
        RES.methods(task_context_methods)
    }
}

impl<'v> values::AllocValue<'v> for TaskContext<'v> {
    fn alloc_value(self, heap: &'v values::Heap) -> values::Value<'v> {
        heap.alloc_complex(self)
    }
}

impl<'v> values::Freeze for TaskContext<'v> {
    type Frozen = FrozenTaskContext;
    fn freeze(self, _freezer: &values::Freezer) -> values::FreezeResult<Self::Frozen> {
        panic!("not implemented")
    }
}

#[derive(Debug, Display, ProvidesStaticType, NoSerialize, Allocative)]
#[display("<task_context>")]
pub struct FrozenTaskContext {
    #[allocative(skip)]
    args: FrozenTaskArgs,
}

starlark_simple_value!(FrozenTaskContext);

#[starlark_value(type = "task_context")]
impl<'v> values::StarlarkValue<'v> for FrozenTaskContext {
    type Canonical = TaskContext<'v>;
}

#[starlark_module]
pub(crate) fn task_context_methods(registry: &mut MethodsBuilder) {
    /// Standard library is the foundation of powerful AXL tasks.
    #[starlark(attribute)]
    fn std<'v>(#[allow(unused)] this: values::Value<'v>) -> starlark::Result<Std> {
        Ok(Std {})
    }

    /// Access to arguments provided by the caller.
    #[starlark(attribute)]
    fn args<'v>(#[allow(unused)] this: values::Value<'v>) -> starlark::Result<TaskArgs<'v>> {
        let ctx = this.downcast_ref_err::<TaskContext>()?;
        // TODO: don't do this.
        Ok(ctx.args.clone())
    }

    /// Access to Aspect Workflows Delivery Service
    #[starlark(attribute)]
    fn delivery<'v>(#[allow(unused)] this: values::Value<'v>) -> starlark::Result<DeliveryModule> {
        // DELIVERY_DB_ENDPOINT
        //
        Ok(DeliveryModule::new())
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
    /// # Example
    ///
    /// ```starlark
    /// # Fetch data from a remote server
    /// data = ctx.http().get("https://example.com/data.json").block()
    /// ```
    fn http<'v>(#[allow(unused)] this: values::Value<'v>) -> starlark::Result<Http> {
        Ok(Http::new())
    }
}
