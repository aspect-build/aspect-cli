//! ConfiguredTask - A task with its fragment type IDs.

use std::cell::RefCell;
use std::path::PathBuf;

use allocative::Allocative;
use anyhow::anyhow;
use derive_more::Display;
use starlark::environment::Methods;
use starlark::environment::MethodsBuilder;
use starlark::environment::MethodsStatic;
use starlark::starlark_module;
use starlark::values;
use starlark::values::Heap;
use starlark::values::NoSerialize;
use starlark::values::OwnedFrozenValue;
use starlark::values::ProvidesStaticType;
use starlark::values::Trace;
use starlark::values::UnpackValue;
use starlark::values::Value;
use starlark::values::ValueError;
use starlark::values::ValueLike;
use starlark::values::list::AllocList;
use starlark::values::list::UnpackList;
use starlark::values::starlark_value;

use crate::engine::task::FrozenTask;
use crate::engine::task::TaskLike;
use crate::eval::EvalError;

/// A task bundled with its fragment type IDs.
///
/// This type:
/// - Has no lifetime parameter (easy to store and pass around)
/// - Uses `OwnedFrozenValue` for frozen values (task definition)
/// - Stores fragment type IDs for fragment map scoping
/// - Is a `StarlarkValue` that config functions can modify via `set_attr`
#[derive(Debug, ProvidesStaticType, Display, NoSerialize, Allocative, Clone)]
#[display("<ConfiguredTask>")]
pub struct ConfiguredTask {
    /// The frozen task definition (contains implementation function)
    #[allocative(skip)]
    pub task_def: OwnedFrozenValue,
    /// Task name (may be overridden by config)
    pub name: RefCell<String>,
    /// Task group (may be overridden by config)
    pub group: RefCell<Vec<String>>,
    /// Fragment type IDs this task opts into
    pub fragment_type_ids: Vec<u64>,
    /// Symbol name in the module
    pub symbol: String,
    /// Path to the .axl file
    pub path: PathBuf,
}

unsafe impl Trace<'_> for ConfiguredTask {
    fn trace(&mut self, _tracer: &values::Tracer<'_>) {
        // OwnedFrozenValue manages its own lifetime.
    }
}

impl ConfiguredTask {
    /// Create a ConfiguredTask from a FrozenModule.
    pub fn from_frozen_module(
        frozen: &starlark::environment::FrozenModule,
        symbol: &str,
        path: PathBuf,
    ) -> Result<Self, EvalError> {
        // Get task definition - returns OwnedFrozenValue which keeps heap alive
        let task_def = frozen
            .get(symbol)
            .map_err(|e| EvalError::UnknownError(anyhow!(e)))?;

        // Verify it's actually a FrozenTask and extract metadata
        let frozen_task = task_def
            .value()
            .downcast_ref::<FrozenTask>()
            .ok_or_else(|| EvalError::UnknownError(anyhow!("symbol '{}' is not a Task", symbol)))?;

        // Use symbol name if task name is empty
        let name = if frozen_task.name.is_empty() {
            symbol.to_string()
        } else {
            frozen_task.name.clone()
        };
        let group = frozen_task.group.clone();
        let fragment_type_ids = frozen_task.fragment_type_ids();

        Ok(ConfiguredTask {
            task_def,
            name: RefCell::new(name),
            group: RefCell::new(group),
            fragment_type_ids,
            symbol: symbol.to_string(),
            path,
        })
    }

    /// Create a ConfiguredTask with known fragment type IDs.
    pub fn new_with_fragments(
        task_def: OwnedFrozenValue,
        name: String,
        group: Vec<String>,
        fragment_type_ids: Vec<u64>,
        symbol: String,
        path: PathBuf,
    ) -> Self {
        ConfiguredTask {
            task_def,
            name: RefCell::new(name),
            group: RefCell::new(group),
            fragment_type_ids,
            symbol,
            path,
        }
    }

    /// Get a reference to the underlying FrozenTask.
    pub fn as_frozen_task(&self) -> Option<&FrozenTask> {
        self.task_def.value().downcast_ref::<FrozenTask>()
    }

    /// Get the task as a TaskLike for introspection.
    pub fn as_task(&self) -> Option<&dyn TaskLike<'_>> {
        self.as_frozen_task().map(|t| t as &dyn TaskLike<'_>)
    }

    /// Get the task implementation function.
    pub fn implementation(&self) -> Option<OwnedFrozenValue> {
        let task = self.as_frozen_task()?;
        Some(self.task_def.map(|_| task.implementation()))
    }

    /// Get the current name.
    pub fn get_name(&self) -> String {
        self.name.borrow().clone()
    }

    /// Get the current group.
    pub fn get_group(&self) -> Vec<String> {
        self.group.borrow().clone()
    }
}

#[starlark_value(type = "ConfiguredTask")]
impl<'v> values::StarlarkValue<'v> for ConfiguredTask {
    fn set_attr(&self, attribute: &str, value: Value<'v>) -> starlark::Result<()> {
        match attribute {
            "name" => {
                self.name.replace(value.to_str());
            }
            "group" => {
                let unpack: UnpackList<String> = UnpackList::unpack_value(value)?
                    .ok_or_else(|| anyhow!("groups must be a list of strings"))?;
                self.group.replace(unpack.items);
            }
            _ => return ValueError::unsupported(self, &format!(".{}=", attribute)),
        };
        Ok(())
    }

    fn get_attr(&self, attribute: &str, heap: &'v Heap) -> Option<Value<'v>> {
        match attribute {
            "name" => Some(heap.alloc_str(&self.name.borrow()).to_value()),
            "group" => Some(heap.alloc(AllocList(self.group.borrow().iter()))),
            "symbol" => Some(heap.alloc_str(&self.symbol).to_value()),
            "path" => Some(heap.alloc_str(&self.path.to_string_lossy()).to_value()),
            _ => None,
        }
    }

    fn dir_attr(&self) -> Vec<String> {
        vec![
            "name".into(),
            "group".into(),
            "symbol".into(),
            "path".into(),
        ]
    }

    fn get_methods() -> Option<&'static Methods> {
        static RES: MethodsStatic = MethodsStatic::new();
        RES.methods(configured_task_methods)
    }
}

#[starlark_module]
fn configured_task_methods(_builder: &mut MethodsBuilder) {
    // Methods can be added here if needed
}

impl<'v> values::AllocValue<'v> for ConfiguredTask {
    fn alloc_value(self, heap: &'v Heap) -> Value<'v> {
        heap.alloc_complex_no_freeze(self)
    }
}
