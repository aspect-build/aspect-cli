use std::cell::RefCell;

use allocative::Allocative;
use anyhow::anyhow;
use derive_more::Display;

use starlark::ErrorKind;
use starlark::collections::SmallSet;
use starlark::environment::Methods;
use starlark::environment::MethodsBuilder;
use starlark::environment::MethodsStatic;

use starlark::starlark_module;
use starlark::values;
use starlark::values::NoSerialize;
use starlark::values::ProvidesStaticType;
use starlark::values::Trace;
use starlark::values::ValueError;
use starlark::values::ValueLike;
use starlark::values::list::AllocList;
use starlark::values::starlark_value;

use super::http::Http;
use super::std::Std;
use super::task::{FrozenTask, Task};
use super::template;
use super::wasm::Wasm;

#[derive(Debug, Clone, ProvidesStaticType, Trace, Display, NoSerialize, Allocative)]
#[display("<TaskReg>")]
pub struct TaskMut<'v> {
    name: String,
    group: Vec<String>,
    
    Unfrozen(RefCell<Task<'v>>),
    Frozen(RefCell<FrozenTask>),
}

impl<'v> TaskMut<'v> {
    fn set_name(&self, name: String) {
        match self {
            TaskMut::Unfrozen(task) => task.borrow_mut().name = name,
            TaskMut::Frozen(task) => task.borrow_mut().name = name,
        }
    }
    fn set_group(&self, group: Vec<String>) {
        match self {
            TaskMut::Unfrozen(task) => task.borrow_mut().group = group,
            TaskMut::Frozen(task) => task.borrow_mut().implementation().to_value(),
        }
    }
}

#[starlark_value(type = "TaskMut")]
impl<'v> values::StarlarkValue<'v> for TaskMut<'v> {
    fn set_attr(&self, attribute: &str, value: values::Value<'v>) -> starlark::Result<()> {
        match attribute {
            "name" => self.set_name(value.to_str()),
            _ => return ValueError::unsupported(self, &format!(".{}=", attribute)),
        };
        Ok(())
    }

    fn dir_attr(&self) -> Vec<String> {
        vec![
            "group".into(),
            "name".into(),
            "description".into(),
            "args".into(),
        ]
    }
}

impl<'v> values::AllocValue<'v> for TaskMut<'v> {
    fn alloc_value(self, heap: &'v values::Heap) -> values::Value<'v> {
        heap.alloc_complex_no_freeze(self)
    }
}

#[derive(Debug, Clone, ProvidesStaticType, Trace, Display, NoSerialize, Allocative)]
#[display("<ConfigContext>")]
pub struct ConfigContext<'v> {
    #[allocative(skip)]
    tasks: SmallSet<TaskMut<'v>>,
}

impl<'v> ConfigContext<'v> {
    pub fn new() -> Self {
        Self {
            tasks: SmallSet::new(),
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
        heap.alloc_complex_no_freeze(self)
    }
}

impl<'v> values::Freeze for ConfigContext<'v> {
    type Frozen = ConfigContext<'v>;
    fn freeze(self, _freezer: &values::Freezer) -> values::FreezeResult<Self::Frozen> {
        Ok(self)
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
    fn tasks<'v>(
        #[allow(unused)] this: values::Value<'v>,
    ) -> starlark::Result<AllocList<SmallSet<TaskMut<'v>>>> {
        let this = this.downcast_ref_err::<ConfigContext>()?;
        Ok(AllocList(this.tasks.clone()))
    }
}
