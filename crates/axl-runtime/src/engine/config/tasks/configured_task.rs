//! ConfiguredTask - A task with its configuration.

use std::cell::RefCell;
use std::path::PathBuf;

use allocative::Allocative;
use anyhow::anyhow;
use derive_more::Display;
use starlark::environment::FrozenModule;
use starlark::environment::Methods;
use starlark::environment::MethodsBuilder;
use starlark::environment::MethodsStatic;
use starlark::eval::Evaluator;
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

/// A task bundled with its configuration.
///
/// This type:
/// - Has no lifetime parameter (easy to store and pass around)
/// - Uses `OwnedFrozenValue` for frozen values (task definition)
/// - Stores config binding for lazy evaluation during config phase
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
    /// The frozen config binding (callable that returns config)
    #[allocative(skip)]
    config_binding: OwnedFrozenValue,
    /// The lazily evaluated config value (mutable, stays on the heap)
    /// SAFETY: This value lives on the same heap as ConfiguredTask
    /// None until evaluate_config() is called
    #[allocative(skip)]
    evaluated_config: RefCell<Option<Value<'static>>>,
    /// Symbol name in the module
    pub symbol: String,
    /// Path to the .axl file
    pub path: PathBuf,
}

unsafe impl Trace<'_> for ConfiguredTask {
    fn trace(&mut self, _tracer: &values::Tracer<'_>) {
        // The evaluated_config value uses 'static lifetime as a workaround, but the actual
        // value lives on the same heap as ConfiguredTask. Since ConfiguredTask
        // is allocated with alloc_complex_no_freeze, the heap manages tracing.
        // The config_binding is an OwnedFrozenValue which manages its own lifetime.
    }
}

impl ConfiguredTask {
    /// Create a ConfiguredTask from a FrozenModule.
    ///
    /// Stores the config binding for lazy evaluation. Call `evaluate_config()`
    /// during config phase when an Evaluator is available.
    pub fn from_frozen_module(
        frozen: &FrozenModule,
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

        // Store the config binding for lazy evaluation
        let config_binding = task_def.map(|_| frozen_task.config());

        Ok(ConfiguredTask {
            task_def,
            name: RefCell::new(name),
            group: RefCell::new(group),
            config_binding,
            evaluated_config: RefCell::new(None),
            symbol: symbol.to_string(),
            path,
        })
    }

    /// Evaluate the config binding and store the result.
    ///
    /// This must be called during config phase when an Evaluator is available.
    /// After this call, `get_config()` will return the evaluated value.
    pub fn evaluate_config<'v>(&self, eval: &mut Evaluator<'v, '_, '_>) -> Result<(), EvalError> {
        // Get the frozen binding value - this has 'static lifetime from OwnedFrozenValue
        let binding = self.config_binding.value();
        // SAFETY: FrozenValue has 'static lifetime, we need to convert to Value<'v>
        // to call eval_function. The actual value is frozen and valid for 'static.
        let binding_value: Value<'v> = unsafe { std::mem::transmute(binding) };

        let config_value: Value<'v> = if binding_value.is_none() {
            binding_value
        } else {
            eval.eval_function(binding_value, &[], &[]).map_err(|e| {
                EvalError::UnknownError(anyhow!("failed to evaluate config binding: {:?}", e))
            })?
        };

        // SAFETY: The config value lives on the evaluator's heap. The lifetime is valid
        // as long as the heap outlives this ConfiguredTask.
        let config: Value<'static> = unsafe { std::mem::transmute(config_value) };
        self.evaluated_config.replace(Some(config));
        Ok(())
    }

    /// Create a ConfiguredTask with an already-evaluated config value.
    ///
    /// Use this when an Evaluator is available (e.g., in task_list.add()).
    pub fn new_with_evaluated_config<'v>(
        task_def: OwnedFrozenValue,
        config_binding: OwnedFrozenValue,
        name: String,
        group: Vec<String>,
        config: Value<'v>,
        symbol: String,
        path: PathBuf,
    ) -> Self {
        // SAFETY: Same as evaluate_config - config lives on the same heap
        let config: Value<'static> = unsafe { std::mem::transmute(config) };
        ConfiguredTask {
            task_def,
            config_binding,
            name: RefCell::new(name),
            group: RefCell::new(group),
            evaluated_config: RefCell::new(Some(config)),
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

    /// Get the current config value.
    ///
    /// Panics if `evaluate_config()` has not been called.
    pub fn get_config<'v>(&self) -> Value<'v> {
        let config = self.evaluated_config.borrow();
        let config = config.expect("config not yet evaluated - call evaluate_config() first");
        // SAFETY: Transmute back to the caller's lifetime - valid because
        // the heap outlives this call
        unsafe { std::mem::transmute(config) }
    }

    /// Try to get the current config value, returning None if not yet evaluated.
    pub fn try_get_config<'v>(&self) -> Option<Value<'v>> {
        let config = *self.evaluated_config.borrow();
        // SAFETY: Transmute back to the caller's lifetime - valid because
        // the heap outlives this call
        config.map(|c| unsafe { std::mem::transmute(c) })
    }

    /// Set the config value.
    pub fn set_config<'v>(&self, value: Value<'v>) {
        // SAFETY: Same lifetime reasoning as get_config
        let value: Value<'static> = unsafe { std::mem::transmute(value) };
        self.evaluated_config.replace(Some(value));
    }

    /// Get the current name.
    pub fn get_name(&self) -> String {
        self.name.borrow().clone()
    }

    /// Get the current group.
    pub fn get_group(&self) -> Vec<String> {
        self.group.borrow().clone()
    }

    /// Get the config binding (for creating new ConfiguredTasks with evaluated config).
    pub fn config_binding(&self) -> &OwnedFrozenValue {
        &self.config_binding
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
            "config" => {
                self.set_config(value);
            }
            _ => return ValueError::unsupported(self, &format!(".{}=", attribute)),
        };
        Ok(())
    }

    fn get_attr(&self, attribute: &str, heap: &'v Heap) -> Option<Value<'v>> {
        match attribute {
            "name" => Some(heap.alloc_str(&self.name.borrow()).to_value()),
            "group" => Some(heap.alloc(AllocList(self.group.borrow().iter()))),
            "config" => Some(self.get_config()),
            "symbol" => Some(heap.alloc_str(&self.symbol).to_value()),
            "path" => Some(heap.alloc_str(&self.path.to_string_lossy()).to_value()),
            _ => None,
        }
    }

    fn dir_attr(&self) -> Vec<String> {
        vec![
            "name".into(),
            "group".into(),
            "config".into(),
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
