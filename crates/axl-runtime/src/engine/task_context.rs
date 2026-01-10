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
use starlark::values::Value;
use starlark::values::ValueLike;

use super::bazel::Bazel;
use super::http::Http;
use super::std::Std;
use super::task_args::FrozenTaskArgs;
use super::task_args::TaskArgs;
use super::template;
use super::wasm::Wasm;

#[derive(Debug, Clone, ProvidesStaticType, Display, Trace, NoSerialize, Allocative)]
#[display("<TaskContext>")]
pub struct TaskContext<'v> {
    pub args: TaskArgs<'v>,
    pub config: values::Value<'v>,
}

impl<'v> TaskContext<'v> {
    pub fn new(args: TaskArgs<'v>, config: values::Value<'v>) -> Self {
        Self { args, config }
    }
}

#[starlark_value(type = "TaskContext")]
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
    fn freeze(self, freezer: &values::Freezer) -> values::FreezeResult<Self::Frozen> {
        Ok(FrozenTaskContext {
            args: self.args.freeze(freezer)?,
            config: self.config.freeze(freezer)?,
        })
    }
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

    /// Access to the task configuration.
    #[starlark(attribute)]
    fn config<'v>(this: values::Value<'v>) -> starlark::Result<values::Value<'v>> {
        let ctx = this.downcast_ref_err::<TaskContext>()?;
        Ok(ctx.config)
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

#[derive(Debug, Display, ProvidesStaticType, NoSerialize, Allocative)]
#[display("<TaskContext>")]
pub struct FrozenTaskContext {
    #[allocative(skip)]
    args: FrozenTaskArgs,
    #[allocative(skip)]
    config: values::FrozenValue,
}

starlark_simple_value!(FrozenTaskContext);

#[starlark_value(type = "TaskContext")]
impl<'v> values::StarlarkValue<'v> for FrozenTaskContext {
    type Canonical = TaskContext<'v>;
}

#[starlark_module]
fn frozen_task_context_methods(registry: &mut MethodsBuilder) {
    #[starlark(attribute)]
    fn std<'v>(#[allow(unused)] this: values::Value<'v>) -> starlark::Result<Std> {
        Ok(Std {})
    }

    #[starlark(attribute)]
    fn args<'v>(this: values::Value<'v>) -> starlark::Result<values::Value<'v>> {
        // TODO: fix this
        // let ctx = this.downcast_ref_err::<FrozenTaskContext>()?;
        // Ok(ctx.args.to_value())
        Ok(Value::new_none())
    }

    #[starlark(attribute)]
    fn config<'v>(this: values::Value<'v>) -> starlark::Result<values::Value<'v>> {
        let ctx = this.downcast_ref_err::<FrozenTaskContext>()?;
        Ok(ctx.config.to_value())
    }

    #[starlark(attribute)]
    fn template<'v>(
        #[allow(unused)] this: values::Value<'v>,
    ) -> starlark::Result<template::Template> {
        Ok(template::Template::new())
    }

    #[starlark(attribute)]
    fn bazel<'v>(#[allow(unused)] this: values::Value<'v>) -> starlark::Result<Bazel> {
        Ok(Bazel {})
    }

    fn http<'v>(#[allow(unused)] this: values::Value<'v>) -> starlark::Result<Http> {
        Ok(Http::new())
    }
}
