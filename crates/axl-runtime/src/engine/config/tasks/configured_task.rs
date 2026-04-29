//! ConfiguredTask - A task with its trait type IDs.

use std::cell::RefCell;
use std::path::PathBuf;

use allocative::Allocative;
use anyhow::anyhow;
use derive_more::Display;
use starlark::collections::SmallMap;
use starlark::environment::Methods;
use starlark::environment::MethodsBuilder;
use starlark::environment::MethodsStatic;
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

use crate::engine::arg::Arg;
use crate::engine::config::tasks::frozen::freeze_value;
use crate::engine::config::tasks::value::task_key;
use crate::engine::task::FrozenTask;
use crate::engine::task::TaskLike;
use crate::engine::types::feature::{to_command_name, validate_command_name};
use crate::eval::EvalError;

/// A task bundled with its trait type IDs.
///
/// `task_def` is a `FrozenValue` pointing to a `FrozenTask`. The frozen heap that
/// owns it is kept alive externally — either by `AxlLoader::loaded_modules` (for
/// tasks discovered in Phase 1) or by the live module's heap registration (for tasks
/// added via `ctx.tasks.add`).
#[derive(Debug, ProvidesStaticType, Display, NoSerialize, Allocative, Clone)]
#[display("<ConfiguredTask>")]
pub struct ConfiguredTask {
    /// The frozen task definition (contains implementation function).
    #[allocative(skip)]
    pub task_def: FrozenValue,
    /// Task name (may be overridden by config)
    pub name: RefCell<String>,
    /// Task group (may be overridden by config)
    pub group: RefCell<Vec<String>>,
    /// Fragment type IDs this task opts into
    pub trait_type_ids: Vec<u64>,
    /// Symbol name in the module
    pub symbol: String,
    /// Path to the .axl file
    pub path: PathBuf,
    /// Config-only attr overrides set from config.axl via `ctx.tasks` iteration.
    #[allocative(skip)]
    pub config_overrides: RefCell<SmallMap<String, OwnedFrozenValue>>,
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
        let owned = frozen
            .get(symbol)
            .map_err(|e| EvalError::UnknownError(anyhow!(e)))?;

        let frozen_task = owned
            .value()
            .downcast_ref::<FrozenTask>()
            .ok_or_else(|| EvalError::UnknownError(anyhow!("symbol '{}' is not a Task", symbol)))?;

        let name = if frozen_task.name.is_empty() {
            to_command_name(symbol)
        } else {
            frozen_task.name.clone()
        };
        validate_command_name(&name, "task").map_err(|e| {
            EvalError::UnknownError(anyhow!(e).context(format!(
                "symbol {:?} in {}",
                symbol,
                path.display()
            )))
        })?;
        let group = frozen_task.group.clone();
        let trait_type_ids = frozen_task.trait_type_ids();
        // Extract the bare FrozenValue; the heap stays alive via AxlLoader::loaded_modules.
        let task_def = owned
            .value()
            .unpack_frozen()
            .expect("value from FrozenModule is always frozen");

        Ok(ConfiguredTask {
            task_def,
            name: RefCell::new(name),
            group: RefCell::new(group),
            trait_type_ids,
            symbol: symbol.to_string(),
            path,
            config_overrides: RefCell::new(SmallMap::new()),
        })
    }

    /// Create a ConfiguredTask from a `FrozenValue` with known metadata.
    ///
    /// The caller is responsible for ensuring the frozen heap that owns `task_def`
    /// remains alive for the duration of the evaluation.
    pub fn new_with_traits(
        task_def: FrozenValue,
        name: String,
        group: Vec<String>,
        trait_type_ids: Vec<u64>,
        symbol: String,
        path: PathBuf,
    ) -> Self {
        ConfiguredTask {
            task_def,
            name: RefCell::new(name),
            group: RefCell::new(group),
            trait_type_ids,
            symbol,
            path,
            config_overrides: RefCell::new(SmallMap::new()),
        }
    }

    /// Get a reference to the underlying FrozenTask.
    pub fn as_frozen_task(&self) -> Option<&FrozenTask> {
        self.task_def.downcast_ref::<FrozenTask>()
    }

    /// Get the task as a TaskLike for introspection.
    pub fn as_task(&self) -> Option<&dyn TaskLike<'_>> {
        self.as_frozen_task().map(|t| t as &dyn TaskLike<'_>)
    }

    /// Get the task implementation as a `FrozenValue`.
    pub fn implementation(&self) -> Option<FrozenValue> {
        Some(self.as_frozen_task()?.implementation())
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
                let name = value.to_str();
                validate_command_name(&name, "task")
                    .map_err(|e| starlark::Error::new_other(anyhow!(e)))?;
                self.name.replace(name);
            }
            "group" => {
                let unpack: UnpackList<String> = UnpackList::unpack_value(value)?
                    .ok_or_else(|| anyhow!("groups must be a list of strings"))?;
                for g in &unpack.items {
                    validate_command_name(g, "group")
                        .map_err(|e| starlark::Error::new_other(anyhow!(e)))?;
                }
                self.group.replace(unpack.items);
            }
            _ => {
                return ValueError::unsupported(self, &format!(".{}=", attribute));
            }
        };
        Ok(())
    }

    fn get_attr(&self, attribute: &str, heap: Heap<'v>) -> Option<Value<'v>> {
        match attribute {
            "name" => Some(heap.alloc_str(&self.name.borrow()).to_value()),
            "group" => Some(heap.alloc(AllocList(self.group.borrow().iter()))),
            "key" => Some(
                heap.alloc_str(&task_key(&self.group.borrow(), &self.name.borrow()))
                    .to_value(),
            ),
            "symbol" => Some(heap.alloc_str(&self.symbol).to_value()),
            "path" => Some(heap.alloc_str(&self.path.to_string_lossy()).to_value()),
            attr => {
                // Return any config override set from config.axl.
                self.config_overrides
                    .borrow()
                    .get(attr)
                    .and_then(|owned| owned.value().unpack_frozen())
                    .map(|fv| fv.to_value())
            }
        }
    }

    fn dir_attr(&self) -> Vec<String> {
        let mut attrs = vec![
            "name".into(),
            "group".into(),
            "key".into(),
            "symbol".into(),
            "path".into(),
        ];
        // Include custom arg names from the underlying task definition.
        if let Some(task) = self.task_def.downcast_ref::<FrozenTask>() {
            for (k, arg) in task.args() {
                if matches!(arg, Arg::Custom { .. }) {
                    attrs.push(k.clone());
                }
            }
        }
        attrs
    }

    fn get_methods() -> Option<&'static Methods> {
        static RES: MethodsStatic = MethodsStatic::new();
        RES.methods(configured_task_methods)
    }
}

#[starlark_module]
fn configured_task_methods(builder: &mut MethodsBuilder) {
    /// Arg accessor for overriding and reading task args from `config.axl`.
    ///
    /// **Writing** stores an override in `config_overrides` that is injected at execution
    /// time (after the task default but before any explicit CLI flag):
    /// ```starlark
    /// ctx.tasks["group/name"].args.message = "hello"
    /// ```
    ///
    /// **Reading** returns the current effective value — the config override if one was set,
    /// otherwise the task definition's default:
    /// ```starlark
    /// current = ctx.tasks["group/name"].args.message  # "hello" or the task's default
    /// ```
    ///
    /// Precedence at execution time: explicit CLI flag > config.axl override > task default.
    #[starlark(attribute)]
    fn args<'v>(this: Value<'v>) -> anyhow::Result<ConfiguredTaskArgs<'v>> {
        this.downcast_ref::<ConfiguredTask>()
            .ok_or_else(|| anyhow!("internal: not a ConfiguredTask"))?;
        Ok(ConfiguredTaskArgs { task: this })
    }
}

impl<'v> values::AllocValue<'v> for ConfiguredTask {
    fn alloc_value(self, heap: Heap<'v>) -> Value<'v> {
        heap.alloc_complex(self)
    }
}

impl Freeze for ConfiguredTask {
    type Frozen = FrozenConfiguredTask;

    fn freeze(self, _freezer: &Freezer) -> Result<Self::Frozen, FreezeError> {
        Ok(FrozenConfiguredTask {
            task_def: self.task_def,
            name: self.name.into_inner(),
            group: self.group.into_inner(),
            trait_type_ids: self.trait_type_ids,
            symbol: self.symbol,
            path: self.path,
            config_overrides: self.config_overrides.into_inner(),
        })
    }
}

/// Frozen version of ConfiguredTask. Read-only after freezing.
#[derive(Debug, ProvidesStaticType, Display, NoSerialize, Allocative)]
#[display("<ConfiguredTask>")]
pub struct FrozenConfiguredTask {
    #[allocative(skip)]
    pub task_def: FrozenValue,
    pub name: String,
    pub group: Vec<String>,
    pub trait_type_ids: Vec<u64>,
    pub symbol: String,
    pub path: PathBuf,
    #[allocative(skip)]
    pub config_overrides: SmallMap<String, OwnedFrozenValue>,
}

unsafe impl Trace<'_> for FrozenConfiguredTask {
    fn trace(&mut self, _tracer: &values::Tracer<'_>) {}
}

starlark_simple_value!(FrozenConfiguredTask);

#[starlark_value(type = "ConfiguredTask")]
impl<'v> values::StarlarkValue<'v> for FrozenConfiguredTask {
    type Canonical = ConfiguredTask;

    fn get_attr(&self, attribute: &str, heap: Heap<'v>) -> Option<Value<'v>> {
        match attribute {
            "name" => Some(heap.alloc_str(&self.name).to_value()),
            "group" => Some(heap.alloc(AllocList(self.group.iter()))),
            "key" => Some(
                heap.alloc_str(&task_key(&self.group, &self.name))
                    .to_value(),
            ),
            "symbol" => Some(heap.alloc_str(&self.symbol).to_value()),
            "path" => Some(heap.alloc_str(&self.path.to_string_lossy()).to_value()),
            attr => self
                .config_overrides
                .get(attr)
                .and_then(|owned| owned.value().unpack_frozen())
                .map(|fv| fv.to_value()),
        }
    }

    fn dir_attr(&self) -> Vec<String> {
        let mut attrs = vec![
            "name".into(),
            "group".into(),
            "key".into(),
            "symbol".into(),
            "path".into(),
        ];
        if let Some(task) = self.task_def.downcast_ref::<FrozenTask>() {
            for (k, arg) in task.args() {
                if matches!(arg, Arg::Custom { .. }) {
                    attrs.push(k.clone());
                }
            }
        }
        attrs
    }
}

// ---------------------------------------------------------------------------
// Default-value materialisation for Arg
// ---------------------------------------------------------------------------

/// Allocate the default value for a `Arg` onto `heap`.
///
/// Used to satisfy reads of unset args in config.axl.
fn cli_arg_default_value<'v>(arg: &Arg, heap: Heap<'v>) -> Value<'v> {
    match arg {
        Arg::Custom { default, .. } => default
            .map(|fv| fv.to_value())
            .unwrap_or_else(|| heap.alloc(starlark::values::none::NoneType)),
        Arg::String { default, .. } => heap.alloc_str(default).to_value(),
        Arg::Boolean { default, .. } => heap.alloc(*default),
        Arg::Int { default, .. } => heap.alloc(*default),
        Arg::UInt { default, .. } => heap.alloc(*default),
        Arg::Positional { default, .. } => {
            let items: Vec<&str> = default
                .as_deref()
                .unwrap_or(&[])
                .iter()
                .map(|s| s.as_str())
                .collect();
            heap.alloc(AllocList(items))
        }
        Arg::TrailingVarArgs { .. } => heap.alloc(AllocList(Vec::<&str>::new())),
        Arg::StringList { default, .. } => heap.alloc(AllocList(
            default.iter().map(|s| s.as_str()).collect::<Vec<_>>(),
        )),
        Arg::BooleanList { default, .. } => heap.alloc(AllocList(default.iter().copied())),
        Arg::IntList { default, .. } => heap.alloc(AllocList(default.iter().copied())),
        Arg::UIntList { default, .. } => heap.alloc(AllocList(default.iter().copied())),
    }
}

// ---------------------------------------------------------------------------
// ConfiguredTaskArgs — mutable accessor returned by `t.args` in config.axl
// ---------------------------------------------------------------------------

/// Accessor returned by `t.args` on a `ConfiguredTask`.
///
/// Holds a back-reference to the task so that arg writes (`t.args.x = v`)
/// are stored in `ConfiguredTask::config_overrides` and injected at execution time.
#[derive(Debug, ProvidesStaticType, Display, NoSerialize, Allocative)]
#[display("<ConfiguredTaskArgs>")]
pub struct ConfiguredTaskArgs<'v> {
    #[allocative(skip)]
    task: Value<'v>,
}

unsafe impl<'v> Trace<'v> for ConfiguredTaskArgs<'v> {
    fn trace(&mut self, tracer: &values::Tracer<'v>) {
        self.task.trace(tracer);
    }
}

#[starlark_value(type = "ConfiguredTaskArgs")]
impl<'v> values::StarlarkValue<'v> for ConfiguredTaskArgs<'v> {
    fn set_attr(&self, attribute: &str, value: Value<'v>) -> starlark::Result<()> {
        let ct = self
            .task
            .downcast_ref::<ConfiguredTask>()
            .ok_or_else(|| starlark::Error::new_other(anyhow!("internal: not a ConfiguredTask")))?;

        let arg = ct
            .task_def
            .downcast_ref::<FrozenTask>()
            .and_then(|t| t.args().get(attribute));

        let arg = match arg {
            Some(a) => a,
            None => return ValueError::unsupported(self, &format!(".{}=", attribute)),
        };

        // Type-check the value against the declared arg type before storing.
        match arg {
            Arg::Custom { .. } => {
                // Detailed type check for Custom args happens at feature invoke time.
            }
            cli_arg => {
                let expected = match cli_arg {
                    Arg::Custom { .. } => unreachable!(),
                    Arg::String { .. } => "string",
                    Arg::Boolean { .. } => "bool",
                    Arg::Int { .. } | Arg::UInt { .. } => "int",
                    // Positional and TrailingVarArgs are list-shaped at runtime
                    // (the impl reads them via list/star-unpack), so config.axl
                    // overrides must also be lists.
                    Arg::Positional { .. }
                    | Arg::StringList { .. }
                    | Arg::TrailingVarArgs { .. } => "list",
                    Arg::BooleanList { .. } | Arg::IntList { .. } | Arg::UIntList { .. } => "list",
                };
                let actual = value.get_type();
                if actual != expected {
                    return Err(starlark::Error::new_other(anyhow!(
                        "task arg {:?}: expected type `{}`, got `{}`",
                        attribute,
                        expected,
                        actual
                    )));
                }
                if let Arg::String {
                    values: Some(valid),
                    ..
                } = cli_arg
                {
                    if let Some(s) = value.unpack_str() {
                        if !valid.iter().any(|v| v == s) {
                            return Err(starlark::Error::new_other(anyhow!(
                                "task arg {:?}: invalid value `{}`, expected one of: {}",
                                attribute,
                                s,
                                valid
                                    .iter()
                                    .map(|v| format!("`{}`", v))
                                    .collect::<Vec<_>>()
                                    .join(", ")
                            )));
                        }
                    }
                }
            }
        }

        let frozen =
            freeze_value(value).map_err(|e| starlark::Error::new_other(anyhow!("{:?}", e)))?;
        ct.config_overrides
            .borrow_mut()
            .insert(attribute.to_owned(), frozen);
        Ok(())
    }

    fn get_attr(&self, attribute: &str, heap: Heap<'v>) -> Option<Value<'v>> {
        let ct = self.task.downcast_ref::<ConfiguredTask>()?;

        // 1. Return the config_override if one has been set.
        if let Some(val) = ct
            .config_overrides
            .borrow()
            .get(attribute)
            .and_then(|owned| owned.value().unpack_frozen())
            .map(|fv| fv.to_value())
        {
            return Some(val);
        }

        // 2. Fall back to the task definition default.
        let task = ct.task_def.downcast_ref::<FrozenTask>()?;
        let arg = task.args().get(attribute)?;
        Some(cli_arg_default_value(arg, heap))
    }

    fn dir_attr(&self) -> Vec<String> {
        if let Some(ct) = self.task.downcast_ref::<ConfiguredTask>() {
            if let Some(task) = ct.task_def.downcast_ref::<FrozenTask>() {
                return task.args().keys().cloned().collect();
            }
        }
        vec![]
    }
}

impl<'v> AllocValue<'v> for ConfiguredTaskArgs<'v> {
    fn alloc_value(self, heap: Heap<'v>) -> Value<'v> {
        heap.alloc_complex(self)
    }
}

impl<'v> Freeze for ConfiguredTaskArgs<'v> {
    type Frozen = FrozenConfiguredTaskArgs;

    fn freeze(self, freezer: &Freezer) -> Result<Self::Frozen, FreezeError> {
        Ok(FrozenConfiguredTaskArgs {
            task: self.task.freeze(freezer)?,
        })
    }
}

/// Frozen version of `ConfiguredTaskArgs` — read-only after freezing.
#[derive(Debug, ProvidesStaticType, Display, NoSerialize, Allocative)]
#[display("<ConfiguredTaskArgs>")]
pub struct FrozenConfiguredTaskArgs {
    #[allocative(skip)]
    task: FrozenValue,
}

starlark_simple_value!(FrozenConfiguredTaskArgs);

#[starlark_value(type = "ConfiguredTaskArgs")]
impl<'v> values::StarlarkValue<'v> for FrozenConfiguredTaskArgs {
    type Canonical = ConfiguredTaskArgs<'v>;

    fn get_attr(&self, attribute: &str, heap: Heap<'v>) -> Option<Value<'v>> {
        let ct = self.task.downcast_ref::<FrozenConfiguredTask>()?;

        // 1. Return the config_override if one has been set.
        if let Some(val) = ct
            .config_overrides
            .get(attribute)
            .and_then(|owned| owned.value().unpack_frozen())
            .map(|fv| fv.to_value())
        {
            return Some(val);
        }

        // 2. Fall back to the task definition default.
        let task = ct.task_def.downcast_ref::<FrozenTask>()?;
        let arg = task.args().get(attribute)?;
        Some(cli_arg_default_value(arg, heap))
    }

    fn dir_attr(&self) -> Vec<String> {
        if let Some(ct) = self.task.downcast_ref::<FrozenConfiguredTask>() {
            if let Some(task) = ct.task_def.downcast_ref::<FrozenTask>() {
                return task.args().keys().cloned().collect();
            }
        }
        vec![]
    }
}
