use std::cell::RefCell;
use std::path::PathBuf;

use allocative::Allocative;
use anyhow::anyhow;
use derive_more::Display;

use starlark::environment::Module;
use starlark::values;
use starlark::values::list::AllocList;
use starlark::values::list::UnpackList;
use starlark::values::starlark_value;
use starlark::values::Heap;
use starlark::values::NoSerialize;
use starlark::values::ProvidesStaticType;
use starlark::values::Trace;
use starlark::values::UnpackValue;
use starlark::values::ValueError;
use starlark::values::ValueLike;

use crate::engine::task::AsTaskLike;
use crate::engine::task::FrozenTask;
use crate::engine::task::Task;
use crate::engine::task::TaskLike;

#[derive(Debug, Clone, ProvidesStaticType, Display, NoSerialize, Allocative)]
#[display("<TaskMut>")]
pub struct TaskMut<'v> {
    #[allocative(skip)]
    pub module: &'v Module,
    pub path: PathBuf,
    pub name: RefCell<String>,
    pub group: RefCell<Vec<String>>,
    // pub config: values::Value<'v>,
    pub original: values::Value<'v>,
}

unsafe impl<'v> Trace<'v> for TaskMut<'v> {
    fn trace(&mut self, tracer: &values::Tracer<'v>) {
        tracer.trace(&mut self.original);
    }
}

impl<'v> TaskMut<'v> {
    pub fn new(
        module: &'v Module,
        path: String,
        name: String,
        group: Vec<String>,
        original: values::Value<'v>,
    ) -> Self {
        TaskMut {
            module,
            path: PathBuf::from(path),
            name: RefCell::new(name),
            group: RefCell::new(group),
            original,
        }
    }

    pub fn as_task(&'v self) -> Option<&'v dyn TaskLike<'v>> {
        if let Some(task) = self.original.downcast_ref::<Task>() {
            return Some(task.as_task());
        } else if let Some(task) = self.original.downcast_ref::<FrozenTask>() {
            return Some(task.as_task());
        } else {
            return None;
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
        eprintln!("{}", attribute == "binding");
        match attribute {
            "name" => Some(heap.alloc_str(self.name.borrow().as_str()).to_value()),
            "group" => Some(heap.alloc(AllocList(self.group.borrow().iter()))),
            "binding" => {
                if let Some(task) = self.original.downcast_ref::<Task>() {
                    Some(task.binding())
                } else if let Some(task) = self.original.downcast_ref::<FrozenTask>() {
                    Some(task.binding())
                } else {
                    None
                }
            }
            _ => None,
        }
    }

    fn dir_attr(&self) -> Vec<String> {
        vec!["name".into(), "group".into(), "binding".into()]
    }
}

impl<'v> values::AllocValue<'v> for TaskMut<'v> {
    fn alloc_value(self, heap: &'v values::Heap) -> values::Value<'v> {
        heap.alloc_complex_no_freeze(self)
    }
}
