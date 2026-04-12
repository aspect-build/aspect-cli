use allocative::Allocative;
use derive_more::Display;

use starlark::environment::Methods;
use starlark::environment::MethodsBuilder;
use starlark::environment::MethodsStatic;

use starlark::StarlarkResultExt;
use starlark::starlark_module;
use starlark::starlark_simple_value;
use starlark::values;
use starlark::values::AllocValue;
use starlark::values::Freeze;
use starlark::values::FreezeError;
use starlark::values::Freezer;
use starlark::values::FrozenValue;
use starlark::values::Heap;
use starlark::values::NoSerialize;
use starlark::values::ProvidesStaticType;
use starlark::values::Trace;
use starlark::values::Tracer;
use starlark::values::ValueLike;
use starlark::values::starlark_value;

use super::super::http::Http;
use super::super::std::Std;
use super::super::template;
use super::super::wasm::Wasm;

use super::tasks::configured_task::ConfiguredTask;
use super::tasks::value::TaskList;

/// Config context for evaluating config.axl files.
///
/// This context holds the list of tasks, the fragment map, and the feature map
/// that config functions can modify.
#[derive(Debug, Clone, ProvidesStaticType, Trace, Display, NoSerialize, Allocative)]
#[display("<ConfigContext>")]
pub struct ConfigContext<'v> {
    #[allocative(skip)]
    tasks: values::Value<'v>,
    #[allocative(skip)]
    trait_map: values::Value<'v>,
    #[allocative(skip)]
    feature_map: values::Value<'v>,
}

impl<'v> ConfigContext<'v> {
    /// Create a new ConfigContext with the given tasks, trait map, and feature map.
    pub fn new(
        tasks: Vec<ConfiguredTask>,
        trait_map: values::Value<'v>,
        feature_map: values::Value<'v>,
        heap: Heap<'v>,
    ) -> Self {
        let tasks: Vec<values::Value<'v>> = tasks
            .into_iter()
            .map(|task| task.alloc_value(heap))
            .collect();
        Self {
            tasks: heap.alloc(TaskList::new(tasks)),
            trait_map,
            feature_map,
        }
    }

    /// Create a new ConfigContext from already-allocated Value<'v> task objects.
    ///
    /// Unlike `new`, this does not re-allocate the tasks; the same Value<'v> pointers
    /// are reused so that mutations during config evaluation are visible to the caller.
    pub fn new_from_values(
        tasks: Vec<values::Value<'v>>,
        trait_map: values::Value<'v>,
        feature_map: values::Value<'v>,
        heap: Heap<'v>,
    ) -> Self {
        Self {
            tasks: heap.alloc(TaskList::new(tasks)),
            trait_map,
            feature_map,
        }
    }

    /// Get references to the tasks.
    pub fn tasks(&self) -> Vec<&ConfiguredTask> {
        self.tasks
            .downcast_ref::<TaskList>()
            .unwrap()
            .content
            .borrow()
            .iter()
            .map(|m| m.downcast_ref::<ConfiguredTask>().unwrap())
            .collect()
    }

    /// Get task values for iteration (used during config evaluation).
    pub fn task_values(&self) -> Vec<values::Value<'v>> {
        self.tasks.downcast_ref::<TaskList>().unwrap().values()
    }

    /// Get the trait map value.
    pub fn trait_map_value(&self) -> values::Value<'v> {
        self.trait_map
    }

    /// Get the feature map value.
    pub fn feature_map_value(&self) -> values::Value<'v> {
        self.feature_map
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
    fn alloc_value(self, heap: values::Heap<'v>) -> values::Value<'v> {
        heap.alloc_complex(self)
    }
}

impl<'v> Freeze for ConfigContext<'v> {
    type Frozen = FrozenConfigContext;

    fn freeze(self, freezer: &Freezer) -> Result<Self::Frozen, FreezeError> {
        Ok(FrozenConfigContext {
            tasks: self.tasks.freeze(freezer)?,
            trait_map: self.trait_map.freeze(freezer)?,
            feature_map: self.feature_map.freeze(freezer)?,
        })
    }
}

/// Frozen version of ConfigContext. Read-only after freezing.
#[derive(Debug, ProvidesStaticType, NoSerialize, Allocative, Display)]
#[display("<ConfigContext>")]
pub struct FrozenConfigContext {
    #[allocative(skip)]
    tasks: FrozenValue,
    #[allocative(skip)]
    trait_map: FrozenValue,
    #[allocative(skip)]
    feature_map: FrozenValue,
}

unsafe impl<'v> Trace<'v> for FrozenConfigContext {
    fn trace(&mut self, _tracer: &Tracer<'v>) {}
}

starlark_simple_value!(FrozenConfigContext);

#[starlark_value(type = "ConfigContext")]
impl<'v> values::StarlarkValue<'v> for FrozenConfigContext {
    type Canonical = ConfigContext<'v>;
}

#[starlark_module]
pub(crate) fn config_context_methods(registry: &mut MethodsBuilder) {
    /// Standard library is the foundation of powerful AXL tasks.
    #[starlark(attribute)]
    fn std<'v>(#[allow(unused)] this: values::Value<'v>) -> anyhow::Result<Std> {
        Ok(Std {})
    }

    /// Expand template files.
    #[starlark(attribute)]
    fn template<'v>(
        #[allow(unused)] this: values::Value<'v>,
    ) -> anyhow::Result<template::Template> {
        Ok(template::Template::new())
    }

    /// EXPERIMENTAL! Run wasm programs within tasks.
    #[starlark(attribute)]
    fn wasm<'v>(#[allow(unused)] this: values::Value<'v>) -> anyhow::Result<Wasm> {
        Ok(Wasm::new())
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
    fn http<'v>(#[allow(unused)] this: values::Value<'v>) -> anyhow::Result<Http> {
        Ok(Http::new())
    }

    #[starlark(attribute)]
    fn tasks<'v>(this: values::Value<'v>) -> anyhow::Result<values::Value<'v>> {
        let ctx = this
            .downcast_ref_err::<ConfigContext>()
            .into_anyhow_result()?;
        Ok(ctx.tasks)
    }

    /// Access to the trait map for configuring trait instances.
    ///
    /// Usage:
    /// ```starlark
    /// ctx.traits[BazelTrait].extra_flags = ["--config=ci"]
    /// ```
    #[starlark(attribute)]
    fn traits<'v>(this: values::Value<'v>) -> anyhow::Result<values::Value<'v>> {
        let ctx = this
            .downcast_ref_err::<ConfigContext>()
            .into_anyhow_result()?;
        Ok(ctx.trait_map)
    }

    /// Access to the feature map for configuring feature instances.
    ///
    /// Usage:
    /// ```starlark
    /// ctx.features[GithubStatusChecks].owner = "myorg"
    /// ctx.features[GithubStatusChecks].enabled = False
    /// ```
    #[starlark(attribute)]
    fn features<'v>(this: values::Value<'v>) -> anyhow::Result<values::Value<'v>> {
        let ctx = this
            .downcast_ref_err::<ConfigContext>()
            .into_anyhow_result()?;
        Ok(ctx.feature_map)
    }
}
