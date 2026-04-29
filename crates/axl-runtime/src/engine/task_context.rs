use allocative::Allocative;
use derive_more::Display;

use starlark::environment::Methods;
use starlark::environment::MethodsBuilder;
use starlark::environment::MethodsStatic;

use starlark::starlark_module;
use starlark::starlark_simple_value;
use starlark::values;
use starlark::values::FrozenValueTyped;
use starlark::values::NoSerialize;
use starlark::values::ProvidesStaticType;
use starlark::values::Trace;
use starlark::values::ValueLike;
use starlark::values::ValueTyped;
use starlark::values::starlark_value;

use super::arguments::{Arguments, FrozenArguments};
use super::aspect::Aspect;
use super::bazel::{Bazel, FrozenBazel};
use super::http::Http;
use super::std::Std;
use super::task_info::TaskInfo;
use super::template::Template;
use super::trait_map::{FrozenTraitMap, TraitMap};

use super::wasm::Wasm;

#[derive(Debug, Clone, ProvidesStaticType, Display, Trace, NoSerialize, Allocative)]
#[display("<TaskContext>")]
pub struct TaskContext<'v> {
    pub args: values::Value<'v>,
    pub traits: values::Value<'v>,
    pub task: values::Value<'v>,
    bazel: values::Value<'v>,
}

impl<'v> TaskContext<'v> {
    pub fn new(
        args: values::Value<'v>,
        traits: values::Value<'v>,
        task: values::Value<'v>,
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
        Ok(FrozenTaskContext {
            args: self.args.freeze(freezer)?,
            traits: self.traits.freeze(freezer)?,
            task: self.task.freeze(freezer)?,
            bazel: self.bazel.freeze(freezer)?,
        })
    }
}

#[starlark_module]
pub(crate) fn task_context_methods(registry: &mut MethodsBuilder) {
    /// Aspect platform integrations — authentication and other hosted services
    /// provided by the Aspect Workflows backend.
    #[starlark(attribute)]
    fn aspect<'v>(#[allow(unused)] this: values::Value<'v>) -> starlark::Result<Aspect> {
        Ok(Aspect {})
    }

    /// The standard library. Gives access to common utilities such as
    /// filesystem, process execution, environment variables, and IO streams.
    #[starlark(attribute)]
    fn std<'v>(#[allow(unused)] this: values::Value<'v>) -> starlark::Result<Std> {
        Ok(Std {})
    }

    /// Identity of the currently running task — its name, group(s),
    /// short human-readable key, and globally unique id.
    #[starlark(attribute)]
    fn task<'v>(this: values::Value<'v>) -> starlark::Result<ValueTyped<'v, TaskInfo>> {
        let ctx = this.downcast_ref_err::<TaskContext>()?;
        Ok(ValueTyped::new_err(ctx.task)?)
    }

    /// Resolved arguments for this task invocation. Read individual values
    /// as `ctx.args.<arg_name>`. Values are produced by merging, in order
    /// of decreasing precedence: explicit CLI flags, `config.axl` overrides,
    /// and the task's declared arg defaults.
    #[starlark(attribute)]
    fn args<'v>(this: values::Value<'v>) -> starlark::Result<ValueTyped<'v, Arguments<'v>>> {
        let ctx = this.downcast_ref_err::<TaskContext>()?;
        Ok(ValueTyped::new_err(ctx.args)?)
    }

    /// Configured trait instances visible to this task. Index by a trait
    /// type to read its fields, e.g. `ctx.traits[GitHub].token`. Only
    /// trait types this task opted into via `task(traits = [...])`
    /// are present.
    #[starlark(attribute)]
    fn traits<'v>(this: values::Value<'v>) -> starlark::Result<ValueTyped<'v, TraitMap<'v>>> {
        let ctx = this.downcast_ref_err::<TaskContext>()?;
        Ok(ValueTyped::new_err(ctx.traits)?)
    }

    /// Render template files by substituting placeholders with values.
    #[starlark(attribute)]
    fn template<'v>(#[allow(unused)] this: values::Value<'v>) -> starlark::Result<Template> {
        Ok(Template::new())
    }

    /// Drive Bazel from a task: run `build`, `test`, `query`, `info`,
    /// parse `.bazelrc`, and consume Build Event Stream output.
    #[starlark(attribute)]
    fn bazel<'v>(this: values::Value<'v>) -> starlark::Result<ValueTyped<'v, Bazel<'v>>> {
        let ctx = this.downcast_ref_err::<TaskContext>()?;
        Ok(ValueTyped::new_err(ctx.bazel)?)
    }

    /// Execute WebAssembly modules within a task.
    ///
    /// EXPERIMENTAL: this surface may change or be removed without notice.
    #[starlark(attribute)]
    fn wasm<'v>(#[allow(unused)] this: values::Value<'v>) -> starlark::Result<Wasm> {
        Ok(Wasm::new())
    }

    /// HTTP client for issuing requests to remote services.
    fn http<'v>(#[allow(unused)] this: values::Value<'v>) -> starlark::Result<Http> {
        Ok(Http::new())
    }
}

#[derive(Debug, Display, ProvidesStaticType, NoSerialize, Allocative)]
#[display("<TaskContext>")]
pub struct FrozenTaskContext {
    args: values::FrozenValue,
    traits: values::FrozenValue,
    task: values::FrozenValue,
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
    /// Aspect platform integrations — authentication and other hosted services
    /// provided by the Aspect Workflows backend.
    #[starlark(attribute)]
    fn aspect<'v>(#[allow(unused)] this: values::Value<'v>) -> starlark::Result<Aspect> {
        Ok(Aspect {})
    }

    /// The standard library. Gives access to common utilities such as
    /// filesystem, process execution, environment variables, and IO streams.
    #[starlark(attribute)]
    fn std<'v>(#[allow(unused)] this: values::Value<'v>) -> starlark::Result<Std> {
        Ok(Std {})
    }

    /// Identity of the currently running task — its name, group(s),
    /// short human-readable key, and globally unique id.
    #[starlark(attribute)]
    fn task<'v>(this: values::Value<'v>) -> starlark::Result<FrozenValueTyped<'v, TaskInfo>> {
        let ctx = this.downcast_ref_err::<FrozenTaskContext>()?;
        Ok(FrozenValueTyped::new_err(ctx.task)?)
    }

    /// Resolved arguments for this task invocation. Read individual values
    /// as `ctx.args.<arg_name>`. Values are produced by merging, in order
    /// of decreasing precedence: explicit CLI flags, `config.axl` overrides,
    /// and the task's declared arg defaults.
    #[starlark(attribute)]
    fn args<'v>(
        this: values::Value<'v>,
    ) -> starlark::Result<FrozenValueTyped<'v, FrozenArguments>> {
        let ctx = this.downcast_ref_err::<FrozenTaskContext>()?;
        Ok(FrozenValueTyped::new_err(ctx.args)?)
    }

    /// Configured trait instances visible to this task. Index by a trait
    /// type to read its fields, e.g. `ctx.traits[GitHub].token`. Only
    /// trait types this task opted into via `task(traits = [...])`
    /// are present.
    #[starlark(attribute)]
    fn traits<'v>(
        this: values::Value<'v>,
    ) -> starlark::Result<FrozenValueTyped<'v, FrozenTraitMap>> {
        let ctx = this.downcast_ref_err::<FrozenTaskContext>()?;
        Ok(FrozenValueTyped::new_err(ctx.traits)?)
    }

    /// Render template files by substituting placeholders with values.
    #[starlark(attribute)]
    fn template<'v>(#[allow(unused)] this: values::Value<'v>) -> starlark::Result<Template> {
        Ok(Template::new())
    }

    /// Drive Bazel from a task: run `build`, `test`, `query`, `info`,
    /// parse `.bazelrc`, and consume Build Event Stream output.
    #[starlark(attribute)]
    fn bazel<'v>(this: values::Value<'v>) -> starlark::Result<FrozenValueTyped<'v, FrozenBazel>> {
        let ctx = this.downcast_ref_err::<FrozenTaskContext>()?;
        Ok(FrozenValueTyped::new_err(ctx.bazel)?)
    }

    /// HTTP client for issuing requests to remote services.
    fn http<'v>(#[allow(unused)] this: values::Value<'v>) -> starlark::Result<Http> {
        Ok(Http::new())
    }

    /// Execute WebAssembly modules within a task.
    ///
    /// EXPERIMENTAL: this surface may change or be removed without notice.
    #[starlark(attribute)]
    fn wasm<'v>(#[allow(unused)] this: values::Value<'v>) -> starlark::Result<Wasm> {
        Ok(Wasm::new())
    }
}
