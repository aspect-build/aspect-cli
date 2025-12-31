use std::cell::RefCell;

use allocative::Allocative;
use derive_more::Display;

use starlark::environment::FrozenModule;
use starlark::environment::Methods;
use starlark::environment::MethodsBuilder;
use starlark::environment::MethodsStatic;

use starlark::starlark_module;
use starlark::values;
use starlark::values::starlark_value;
use starlark::values::AllocValue;
use starlark::values::Heap;
use starlark::values::NoSerialize;
use starlark::values::ProvidesStaticType;
use starlark::values::Trace;
use starlark::values::ValueLike;
use starlark::values::ValueOfUnchecked;

use crate::engine::config::task_list::value::MutableTaskList;
use crate::engine::config::task_list::value::TaskListGen;

use super::super::http::Http;
use super::super::std::Std;
use super::super::template;
use super::super::wasm::Wasm;

use super::task_list::r#ref::TaskListRef;
use super::task_list::task_mut::TaskMut;
use super::task_list::value::TaskList;

#[derive(Debug, Clone, ProvidesStaticType, Trace, Display, NoSerialize, Allocative)]
#[display("<ConfigContext>")]
pub struct ConfigContext<'v> {
    #[allocative(skip)]
    tasks: values::Value<'v>,
    #[allocative(skip)]
    config_modules: RefCell<Vec<FrozenModule>>,
}

impl<'v> ConfigContext<'v> {
    pub fn new(tasks: Vec<TaskMut<'v>>, heap: &'v Heap) -> Self {
        let tasks: Vec<values::Value<'v>> = tasks
            .into_iter()
            .map(|task| task.alloc_value(heap))
            .collect();
        let x = TaskListGen(RefCell::new(TaskList::new(tasks)));
        Self {
            tasks: heap.alloc_complex_no_freeze(x),
            config_modules: RefCell::new(vec![]),
        }
    }

    pub fn tasks(&'v self) -> Vec<&'v TaskMut<'v>> {
        let list = self.tasks.downcast_ref::<MutableTaskList>().unwrap();
        list.0
            .borrow()
            .content
            .iter()
            .map(|m| m.downcast_ref::<TaskMut>().unwrap())
            .collect()
    }

    pub fn add_config_module(&self, module: FrozenModule) {
        self.config_modules.borrow_mut().push(module);
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
        heap.alloc_complex_no_freeze(self)
    }
}

impl<'v> values::Freeze for ConfigContext<'v> {
    type Frozen = ConfigContext<'v>;
    fn freeze(self, _freezer: &values::Freezer) -> values::FreezeResult<Self::Frozen> {
        panic!("not implemented")
    }
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

    #[starlark(attribute)]
    fn tasks<'v>(
        #[allow(unused)] this: values::Value<'v>,
    ) -> anyhow::Result<ValueOfUnchecked<'v, &'v TaskListRef<'v>>> {
        let this = this.downcast_ref_err::<ConfigContext>()?;
        Ok(ValueOfUnchecked::new(this.tasks))
    }
}
