use std::cell::Cell;
use std::cell::RefCell;

use allocative::Allocative;
use anyhow::anyhow;
use derive_more::Display;

use dupe::Dupe;
use starlark::environment::Methods;
use starlark::environment::MethodsBuilder;
use starlark::environment::MethodsStatic;

use starlark::eval::Evaluator;
use starlark::starlark_module;
use starlark::values;
use starlark::values::list::AllocList;
use starlark::values::list::ListRef;
use starlark::values::list::UnpackList;
use starlark::values::none::NoneType;
use starlark::values::starlark_value;
use starlark::values::AllocValue;
use starlark::values::Heap;
use starlark::values::NoSerialize;
use starlark::values::ProvidesStaticType;
use starlark::values::Trace;
use starlark::values::UnpackValue;
use starlark::values::ValueError;
use starlark::values::ValueLike;

use crate::engine::task::Task;

use super::http::Http;
use super::std::Std;
use super::template;
use super::wasm::Wasm;

#[derive(Debug, Clone, ProvidesStaticType, Trace, Display, NoSerialize, Allocative, Dupe)]
#[display("<TaskList>")]
pub struct TaskList<'v> {
    #[allocative(skip)]
    inner: Cell<values::ValueOfUnchecked<'v, &'v ListRef<'v>>>,
}

impl<'v> TaskList<'v> {}

#[starlark_value(type = "TaskList")]
impl<'v> values::StarlarkValue<'v> for TaskList<'v> {
    fn iterate_collect(&self, _heap: &'v Heap) -> starlark::Result<Vec<values::Value<'v>>> {
        let value = self.inner.get().get();
        let list = UnpackList::<values::Value<'v>>::unpack_value(value)?;
        Ok(list.unwrap().items)
    }

    fn get_methods() -> Option<&'static Methods> {
        static RES: MethodsStatic = MethodsStatic::new();
        RES.methods(task_list_methods)
    }
}

#[starlark_module]
pub(crate) fn task_list_methods(registry: &mut MethodsBuilder) {
    fn add<'v>(
        #[allow(unused)] this: values::Value<'v>,
        #[starlark[require = pos]] task: values::Value<'v>,
        #[starlark[require = named]] name: String,

        eval: &mut Evaluator<'v, '_, '_>,
    ) -> starlark::Result<NoneType> {
        let mut this = this.downcast_ref_err::<TaskList>()?;
        let this = this.inner.get_mut();
        Ok(NoneType)
    }
}

impl<'v> values::AllocValue<'v> for TaskList<'v> {
    fn alloc_value(self, heap: &'v values::Heap) -> values::Value<'v> {
        heap.alloc_complex_no_freeze(self)
    }
}

#[derive(Debug, Clone, ProvidesStaticType, Trace, Display, NoSerialize, Allocative)]
#[display("<TaskMut>")]
pub struct TaskMut<'v> {
    name: RefCell<String>,
    group: RefCell<Vec<String>>,
    // config:
    original: values::Value<'v>,
}

impl<'v> TaskMut<'v> {
    pub fn new(name: String, group: Vec<String>, original: values::Value<'v>) -> Self {
        TaskMut {
            name: RefCell::new(name),
            group: RefCell::new(group),
            original,
        }
    }
}

#[starlark_value(type = "TaskMut")]
impl<'v> values::StarlarkValue<'v> for TaskMut<'v> {
    fn set_attr(&self, attribute: &str, value: values::Value<'v>) -> starlark::Result<()> {
        match attribute {
            "name" => {
                self.name.replace(value.to_str());
            }
            "group" => {
                let unpack: UnpackList<String> = UnpackList::unpack_value(value)?
                    .ok_or(anyhow!("groups must be a list of strings"))?;
                self.group.replace(unpack.items);
            }
            _ => return ValueError::unsupported(self, &format!(".{}=", attribute)),
        };
        Ok(())
    }

    fn get_attr(&self, attribute: &str, heap: &'v Heap) -> Option<values::Value<'v>> {
        match attribute {
            "name" => Some(heap.alloc_str(self.name.borrow().as_str()).to_value()),
            "group" => Some(heap.alloc(AllocList(self.group.borrow().iter()))),
            _ => None,
        }
    }

    fn dir_attr(&self) -> Vec<String> {
        vec![
            "group".into(),
            "name".into(),
            "args".into(),
            "config".into(),
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
    tasks: TaskList<'v>,
}

impl<'v> ConfigContext<'v> {
    pub fn new(tasks: Vec<TaskMut<'v>>, heap: &'v Heap) -> Self {
        let tasks = heap.alloc_typed_unchecked(AllocList(
            tasks.into_iter().map(|task| task.alloc_value(heap)),
        ));
        Self {
            tasks: TaskList {
                inner: Cell::new(tasks.cast()),
            },
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
    fn tasks<'v>(#[allow(unused)] this: values::Value<'v>) -> starlark::Result<TaskList<'v>> {
        let this = this.downcast_ref_err::<ConfigContext>()?;
        Ok(this.tasks.dupe())
    }
}
