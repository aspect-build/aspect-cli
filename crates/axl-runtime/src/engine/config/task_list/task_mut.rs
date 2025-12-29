use std::cell::RefCell;
use std::path::PathBuf;

use allocative::Allocative;
use anyhow::anyhow;
use derive_more::Display;

use starlark::environment::FrozenModule;
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
use starlark::values::Value;
use starlark::values::ValueError;
use starlark::values::ValueLike;

use crate::engine::task::AsTaskLike;
use crate::engine::task::FrozenTask;
use crate::engine::task::Task;
use crate::engine::task::TaskLike;

#[derive(Debug, Clone, ProvidesStaticType, Display, NoSerialize, Allocative)]
#[display("<TaskMut>")]
pub struct TaskMut<'v> {
    pub name: RefCell<String>,
    pub group: RefCell<Vec<String>>,
    pub config: RefCell<Value<'v>>,
    pub symbol: String,
    pub path: PathBuf,
    #[allocative(skip)]
    pub module: &'v Module,
    #[allocative(skip)]
    pub frozen_config_module: RefCell<Option<FrozenModule>>,
}

unsafe impl<'v> Trace<'v> for TaskMut<'v> {
    fn trace(&mut self, tracer: &values::Tracer<'v>) {
        tracer.trace(&mut *self.config.borrow_mut());
    }
}

impl<'v> TaskMut<'v> {
    pub fn new(
        module: &'v Module,
        symbol: String,
        path: String,
        name: String,
        group: Vec<String>,
    ) -> Self {
        TaskMut {
            name: RefCell::new(name),
            group: RefCell::new(group),
            config: RefCell::new(Value::new_none()),
            symbol,
            path: PathBuf::from(path),
            module,
            frozen_config_module: RefCell::new(None),
        }
    }

    pub fn as_task(&'v self) -> Option<&'v dyn TaskLike<'v>> {
        let original = self
            .module
            .get(&self.symbol)
            .expect("symbol should have been defined.");
        if let Some(task) = original.downcast_ref::<Task<'v>>() {
            return Some(task.as_task());
        } else if let Some(task) = original.downcast_ref::<FrozenTask>() {
            return Some(task.as_task());
        } else {
            return None;
        }
    }

    pub fn initial_config(&self) -> Value<'v> {
        let original = self
            .module
            .get(&self.symbol)
            .expect("symbol should have been defined.");
        if let Some(task) = original.downcast_ref::<Task<'v>>() {
            task.config
        } else if let Some(task) = original.downcast_ref::<FrozenTask>() {
            task.config.to_value()
        } else {
            Value::new_none()
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
            "config" => {
                // Use a frozen version of the config value passed in so that it can be safely referenced across
                // Starlark modules as the value is set in the config module but will used by the task that owns
                // the config which is most often in a different module.
                let temp_module = Module::new();
                let short_value: Value = unsafe { std::mem::transmute(value) };
                temp_module.set("temp", short_value);
                let frozen = temp_module.freeze().expect("freeze failed");
                let frozen_val: Value<'v> =
                    unsafe { std::mem::transmute(frozen.get("temp").expect("get").value()) };
                self.config.replace(frozen_val);
                // Store the frozen module so the frozen heap that backs the config value is not lost.
                *self.frozen_config_module.borrow_mut() = Some(frozen);
            }
            _ => return ValueError::unsupported(self, &format!(".{}=", attribute)),
        };
        Ok(())
    }

    fn get_attr(&self, attribute: &str, heap: &'v Heap) -> Option<values::Value<'v>> {
        match attribute {
            "name" => Some(heap.alloc_str(self.name.borrow().as_str()).to_value()),
            "group" => Some(heap.alloc(AllocList(self.group.borrow().iter()))),
            "config" => Some(*self.config.borrow()),
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
