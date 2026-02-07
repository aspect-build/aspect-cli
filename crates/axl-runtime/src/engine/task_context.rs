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
use starlark::values::Trace;
use starlark::values::ValueLike;
use starlark::values::starlark_value;

use super::bazel::Bazel;
use super::http::Http;
use super::std::Std;
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
        // Freeze args by first freezing TaskArgs to FrozenTaskArgs, then allocating on frozen heap
        let frozen_args = self.args.freeze(freezer)?;
        let args_value = freezer.frozen_heap().alloc_simple(frozen_args);

        Ok(FrozenTaskContext {
            args: args_value,
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
    args: values::FrozenValue,
    #[allocative(skip)]
    config: values::FrozenValue,
}

starlark_simple_value!(FrozenTaskContext);

#[starlark_value(type = "TaskContext")]
impl<'v> values::StarlarkValue<'v> for FrozenTaskContext {
    type Canonical = TaskContext<'v>;

    fn get_methods() -> Option<&'static Methods> {
        static RES: MethodsStatic = MethodsStatic::new();
        RES.methods(frozen_task_context_methods)
    }
}

#[starlark_module]
fn frozen_task_context_methods(registry: &mut MethodsBuilder) {
    #[starlark(attribute)]
    fn std<'v>(#[allow(unused)] this: values::Value<'v>) -> starlark::Result<Std> {
        Ok(Std {})
    }

    #[starlark(attribute)]
    fn args<'v>(this: values::Value<'v>) -> starlark::Result<values::Value<'v>> {
        let ctx = this.downcast_ref_err::<FrozenTaskContext>()?;
        Ok(ctx.args.to_value())
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

    /// EXPERIMENTAL! Run wasm programs within tasks.
    ///
    /// The frozen context is passed directly from `this` - no need to go through
    /// the store since `this` IS the frozen TaskContext.
    #[starlark(attribute)]
    fn wasm<'v>(this: values::Value<'v>) -> starlark::Result<Wasm> {
        let frozen_ctx = this
            .unpack_frozen()
            .ok_or_else(|| anyhow::anyhow!("TaskContext must be frozen for wasm access"))?;
        Ok(Wasm::with_context(frozen_ctx))
    }
}
