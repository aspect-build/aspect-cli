use allocative::Allocative;
use derive_more::Display;

use starlark::environment::Methods;
use starlark::environment::MethodsBuilder;
use starlark::environment::MethodsStatic;

use starlark::StarlarkResultExt;
use starlark::starlark_module;
use starlark::starlark_simple_value;
use starlark::values;
use starlark::values::NoSerialize;
use starlark::values::ProvidesStaticType;
use starlark::values::Trace;
use starlark::values::ValueLike;
use starlark::values::starlark_value;

use super::aspect::Aspect;
use super::http::Http;
use super::std::Std;
use super::task_args::TaskArgs;
use super::task_info::TaskInfo;
use super::template;
use super::wasm::Wasm;

#[derive(Debug, Clone, ProvidesStaticType, Display, Trace, NoSerialize, Allocative)]
#[display("<TaskContext>")]
pub struct TaskContext<'v> {
    pub args: TaskArgs<'v>,
    pub traits: values::Value<'v>,
    #[trace(unsafe_ignore)]
    pub task: TaskInfo,
    bazel: values::Value<'v>,
}

impl<'v> TaskContext<'v> {
    pub fn new(
        args: TaskArgs<'v>,
        traits: values::Value<'v>,
        task: TaskInfo,
        bazel: values::Value<'v>,
    ) -> Self {
        Self {
            args,
            traits,
            task,
            bazel,
        }
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
    fn alloc_value(self, heap: values::Heap<'v>) -> values::Value<'v> {
        heap.alloc_complex(self)
    }
}

impl<'v> values::Freeze for TaskContext<'v> {
    type Frozen = FrozenTaskContext;
    fn freeze(self, freezer: &values::Freezer) -> values::FreezeResult<Self::Frozen> {
        let frozen_args = self.args.freeze(freezer)?;
        let args_value = freezer.frozen_heap().alloc_simple(frozen_args);

        Ok(FrozenTaskContext {
            args: args_value,
            traits: self.traits.freeze(freezer)?,
            task: self.task,
            bazel: self.bazel.freeze(freezer)?,
        })
    }
}

#[starlark_module]
pub(crate) fn task_context_methods(registry: &mut MethodsBuilder) {
    /// Aspect platform APIs (auth, etc.).
    #[starlark(attribute)]
    fn aspect<'v>(#[allow(unused)] this: values::Value<'v>) -> anyhow::Result<Aspect> {
        Ok(Aspect {})
    }

    /// Standard library is the foundation of powerful AXL tasks.
    #[starlark(attribute)]
    fn std<'v>(#[allow(unused)] this: values::Value<'v>) -> anyhow::Result<Std> {
        Ok(Std {})
    }

    /// Identity information for this task (name and group).
    #[starlark(attribute)]
    fn task<'v>(this: values::Value<'v>) -> anyhow::Result<TaskInfo> {
        let ctx = this
            .downcast_ref_err::<TaskContext>()
            .into_anyhow_result()?;
        Ok(ctx.task.clone())
    }

    /// Access to arguments provided by the caller.
    #[starlark(attribute)]
    fn args<'v>(#[allow(unused)] this: values::Value<'v>) -> anyhow::Result<TaskArgs<'v>> {
        let ctx = this
            .downcast_ref_err::<TaskContext>()
            .into_anyhow_result()?;
        // TODO: don't do this.
        Ok(ctx.args.clone())
    }

    /// Access to the trait map for reading configured trait values.
    #[starlark(attribute)]
    fn traits<'v>(this: values::Value<'v>) -> anyhow::Result<values::Value<'v>> {
        let ctx = this
            .downcast_ref_err::<TaskContext>()
            .into_anyhow_result()?;
        Ok(ctx.traits)
    }

    /// Expand template files.
    #[starlark(attribute)]
    fn template<'v>(
        #[allow(unused)] this: values::Value<'v>,
    ) -> anyhow::Result<template::Template> {
        Ok(template::Template::new())
    }

    /// Access to Bazel functionality.
    #[starlark(attribute)]
    fn bazel<'v>(this: values::Value<'v>) -> anyhow::Result<values::Value<'v>> {
        let ctx = this
            .downcast_ref_err::<TaskContext>()
            .into_anyhow_result()?;
        Ok(ctx.bazel)
    }

    /// EXPERIMENTAL! Run wasm programs within tasks.
    #[starlark(attribute)]
    fn wasm<'v>(#[allow(unused)] this: values::Value<'v>) -> anyhow::Result<Wasm> {
        Ok(Wasm::new())
    }

    fn http<'v>(#[allow(unused)] this: values::Value<'v>) -> anyhow::Result<Http> {
        Ok(Http::new())
    }
}

#[derive(Debug, Display, ProvidesStaticType, NoSerialize, Allocative)]
#[display("<TaskContext>")]
pub struct FrozenTaskContext {
    #[allocative(skip)]
    args: values::FrozenValue,
    #[allocative(skip)]
    traits: values::FrozenValue,
    task: TaskInfo,
    #[allocative(skip)]
    bazel: values::FrozenValue,
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
    fn aspect<'v>(#[allow(unused)] this: values::Value<'v>) -> anyhow::Result<Aspect> {
        Ok(Aspect {})
    }

    #[starlark(attribute)]
    fn std<'v>(#[allow(unused)] this: values::Value<'v>) -> anyhow::Result<Std> {
        Ok(Std {})
    }

    #[starlark(attribute)]
    fn task<'v>(this: values::Value<'v>) -> anyhow::Result<TaskInfo> {
        let ctx = this
            .downcast_ref_err::<FrozenTaskContext>()
            .into_anyhow_result()?;
        Ok(ctx.task.clone())
    }

    #[starlark(attribute)]
    fn args<'v>(this: values::Value<'v>) -> anyhow::Result<values::Value<'v>> {
        let ctx = this
            .downcast_ref_err::<FrozenTaskContext>()
            .into_anyhow_result()?;
        Ok(ctx.args.to_value())
    }

    #[starlark(attribute)]
    fn traits<'v>(this: values::Value<'v>) -> anyhow::Result<values::Value<'v>> {
        let ctx = this
            .downcast_ref_err::<FrozenTaskContext>()
            .into_anyhow_result()?;
        Ok(ctx.traits.to_value())
    }

    #[starlark(attribute)]
    fn template<'v>(
        #[allow(unused)] this: values::Value<'v>,
    ) -> anyhow::Result<template::Template> {
        Ok(template::Template::new())
    }

    #[starlark(attribute)]
    fn bazel<'v>(this: values::Value<'v>) -> anyhow::Result<values::Value<'v>> {
        let ctx = this
            .downcast_ref_err::<FrozenTaskContext>()
            .into_anyhow_result()?;
        Ok(ctx.bazel.to_value())
    }

    fn http<'v>(#[allow(unused)] this: values::Value<'v>) -> anyhow::Result<Http> {
        Ok(Http::new())
    }

    #[starlark(attribute)]
    fn wasm<'v>(#[allow(unused)] this: values::Value<'v>) -> anyhow::Result<Wasm> {
        Ok(Wasm::new())
    }
}
