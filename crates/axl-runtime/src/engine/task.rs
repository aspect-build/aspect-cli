use std::cell::RefCell;
use std::path::PathBuf;

use allocative::Allocative;
use derive_more::Display;
use starlark::collections::SmallMap;
use starlark::environment::GlobalsBuilder;
use starlark::starlark_module;
use starlark::starlark_simple_value;
use starlark::typing::ParamIsRequired;
use starlark::typing::ParamSpec;
use starlark::values;
use starlark::values::FrozenValue;
use starlark::values::Heap;
use starlark::values::NoSerialize;
use starlark::values::ProvidesStaticType;
use starlark::values::StarlarkValue;
use starlark::values::Trace;
use starlark::values::Value;
use starlark::values::ValueLike;
use starlark::values::list::UnpackList;
use starlark::values::none::NoneType;
use starlark::values::starlark_value;
use starlark::values::typing::StarlarkCallableParamSpec;

use super::arg::Arg;
use super::arguments::Arguments;
use super::names::{to_command_name, to_display_name, validate_arg_name, validate_command_name};
use super::store::Env;
use super::task_context::TaskContext;
use super::r#trait::{FrozenTraitType, TraitType, extract_trait_type_id};

pub const MAX_TASK_GROUPS: usize = 5;

pub trait TaskLike<'v> {
    /// Full arg map (CLI-exposed + Custom).
    fn args(&self) -> &SmallMap<String, Arg>;
    /// One-line summary shown in the task list. Empty means use the "defined in" fallback.
    fn summary(&self) -> &String;
    /// Extended description shown only in `--help`, after the summary. Empty means omit.
    fn description(&self) -> &String;
    fn display_name(&self) -> String;
    fn group(&self) -> &Vec<String>;
    fn name(&self) -> String;
    /// Absolute path to the .axl file the task was defined in.
    fn path(&self) -> &PathBuf;
    /// `Arguments` value carrying config.axl overrides for this task.
    /// Live tasks return their mutable store directly; frozen tasks return
    /// the frozen variant lifted to the live heap via `to_value()`.
    fn overrides(&self) -> Value<'v>;
    /// Implementation function lifted to the live heap via `to_value()`.
    fn implementation(&self) -> Value<'v>;
    /// Trait type ids this task opts into (via the `traits = [...]` kwarg).
    fn trait_type_ids(&self) -> Vec<u64>;

    /// Returns only the CLI-exposed args (non-Custom entries), as (name, &Arg) pairs.
    fn cli_args(&self) -> Vec<(&str, &Arg)> {
        self.args()
            .iter()
            .filter(|(_, v)| v.is_cli_exposed())
            .map(|(k, v)| (k.as_str(), v))
            .collect()
    }
}

/// Cast helper: borrow anything carrying a task as `&dyn TaskLike<'v>`.
///
/// Implemented for `Task` / `FrozenTask` (trivial upcast) and for `Value<'v>`
/// (downcasts to whichever variant the value holds). Lets callers iterate
/// `TaskMap` entries without caring whether they are live or frozen.
pub trait AsTaskLike<'v> {
    fn as_task(&self) -> &dyn TaskLike<'v>;
}

impl<'v> AsTaskLike<'v> for Task<'v> {
    fn as_task(&self) -> &dyn TaskLike<'v> {
        self
    }
}

impl<'v> AsTaskLike<'v> for FrozenTask {
    fn as_task(&self) -> &dyn TaskLike<'v> {
        self
    }
}

impl<'v> AsTaskLike<'v> for Value<'v> {
    fn as_task(&self) -> &dyn TaskLike<'v> {
        if let Some(t) = self.downcast_ref::<Task<'v>>() {
            return t;
        }
        if let Some(t) = self.downcast_ref::<FrozenTask>() {
            return t;
        }
        panic!("expected a task value, got '{}'", self.get_type());
    }
}

#[derive(Debug, Clone, ProvidesStaticType, Display, Trace, NoSerialize, Allocative)]
#[display("<Task>")]
pub struct Task<'v> {
    r#impl: values::Value<'v>,
    #[allocative(skip)]
    pub(super) args: SmallMap<String, Arg>,
    pub(super) summary: String,
    pub(super) description: String,
    pub(super) display_name: RefCell<String>,
    pub(super) group: Vec<String>,
    pub(super) name: RefCell<String>,
    pub(super) traits: Vec<values::Value<'v>>,
    pub(super) path: PathBuf,
    /// Mutable override store for `ctx.tasks["k"].args.foo = ...`.
    /// Always points to an `Arguments` value on the same heap.
    pub(super) overrides: Value<'v>,
}

impl<'v> Task<'v> {
    pub fn implementation(&self) -> values::Value<'v> {
        self.r#impl
    }
    pub fn args(&self) -> &SmallMap<String, Arg> {
        &self.args
    }
    pub fn summary(&self) -> &String {
        &self.summary
    }
    pub fn description(&self) -> &String {
        &self.description
    }
    pub fn display_name(&self) -> String {
        self.display_name.borrow().clone()
    }
    pub fn group(&self) -> &Vec<String> {
        &self.group
    }
    pub fn name(&self) -> String {
        self.name.borrow().clone()
    }
    pub fn traits(&self) -> &[values::Value<'v>] {
        &self.traits
    }
    pub fn path(&self) -> &PathBuf {
        &self.path
    }
    /// `Arguments` value carrying config.axl overrides for this task.
    pub fn overrides(&self) -> Value<'v> {
        self.overrides
    }
    /// Get trait type IDs this task opts into.
    pub fn trait_type_ids(&self) -> Vec<u64> {
        self.traits
            .iter()
            .filter_map(|v| extract_trait_type_id(*v))
            .collect()
    }

    /// Allocate a live `Task` on `heap` from a `FrozenTask` reference.
    ///
    /// Used by `MultiPhaseEval` to lift a task out of its per-file frozen
    /// module and onto the shared heap, where config.axl can mutate its
    /// args via `ctx.tasks["k"].args.foo = ...`. A fresh empty `Arguments`
    /// override store is allocated.
    pub fn from_frozen(frozen_value: FrozenValue, heap: Heap<'v>) -> Self {
        let frozen = frozen_value
            .downcast_ref::<FrozenTask>()
            .expect("from_frozen called with non-FrozenTask value");
        let overrides = heap.alloc(Arguments::new());
        let traits: Vec<Value<'v>> = frozen.traits.iter().map(|fv| fv.to_value()).collect();
        Task {
            r#impl: frozen.r#impl.to_value(),
            args: frozen.args.clone(),
            summary: frozen.summary.clone(),
            description: frozen.description.clone(),
            display_name: RefCell::new(frozen.display_name.clone()),
            group: frozen.group.clone(),
            name: RefCell::new(frozen.name.clone()),
            traits,
            path: frozen.path.clone(),
            overrides,
        }
    }
}

impl<'v> TaskLike<'v> for Task<'v> {
    fn args(&self) -> &SmallMap<String, Arg> {
        &self.args
    }
    fn summary(&self) -> &String {
        &self.summary
    }
    fn description(&self) -> &String {
        &self.description
    }
    fn display_name(&self) -> String {
        self.display_name.borrow().clone()
    }
    fn group(&self) -> &Vec<String> {
        &self.group
    }
    fn name(&self) -> String {
        self.name.borrow().clone()
    }
    fn path(&self) -> &PathBuf {
        &self.path
    }
    fn implementation(&self) -> Value<'v> {
        self.r#impl
    }
    fn trait_type_ids(&self) -> Vec<u64> {
        self.traits
            .iter()
            .filter_map(|v| extract_trait_type_id(*v))
            .collect()
    }
    fn overrides(&self) -> Value<'v> {
        self.overrides
    }
}

#[starlark_value(type = "Task")]
impl<'v> StarlarkValue<'v> for Task<'v> {
    fn export_as(
        &self,
        variable_name: &str,
        _eval: &mut starlark::eval::Evaluator<'v, '_, '_>,
    ) -> starlark::Result<()> {
        // Snake_case Starlark variables are normalized to kebab-case command names.
        // Tasks are addressed by `ctx.tasks["group/name"]`, mirroring how Bazel
        // identifies build targets — string-keyed by their canonical CLI path.
        let kebab = to_command_name(variable_name);
        validate_command_name(&kebab, "task")
            .map_err(|e| starlark::Error::new_other(anyhow::anyhow!(e)))?;
        let mut name = self.name.borrow_mut();
        if name.is_empty() {
            *name = kebab;
        }
        let mut display_name = self.display_name.borrow_mut();
        if display_name.is_empty() {
            *display_name = to_display_name(&name);
        }
        Ok(())
    }

    fn get_attr(&self, attribute: &str, _heap: Heap<'v>) -> Option<Value<'v>> {
        match attribute {
            "args" => Some(self.overrides),
            _ => None,
        }
    }

    fn has_attr(&self, attribute: &str, _heap: Heap<'v>) -> bool {
        attribute == "args"
    }

    fn dir_attr(&self) -> Vec<String> {
        vec!["args".to_owned()]
    }
}

impl<'v> values::AllocValue<'v> for Task<'v> {
    fn alloc_value(self, heap: values::Heap<'v>) -> Value<'v> {
        heap.alloc_complex(self)
    }
}

impl<'v> values::Freeze for Task<'v> {
    type Frozen = FrozenTask;
    fn freeze(self, freezer: &values::Freezer) -> values::FreezeResult<Self::Frozen> {
        let frozen_impl = self.r#impl.freeze(freezer)?;
        let frozen_traits: Result<Vec<_>, _> =
            self.traits.iter().map(|f| f.freeze(freezer)).collect();
        Ok(FrozenTask {
            args: self.args,
            r#impl: frozen_impl,
            summary: self.summary,
            description: self.description,
            display_name: self.display_name.into_inner(),
            group: self.group,
            name: self.name.into_inner(),
            traits: frozen_traits?,
            path: self.path,
            overrides: self.overrides.freeze(freezer)?,
        })
    }
}

#[derive(Debug, Display, Clone, ProvidesStaticType, Trace, NoSerialize, Allocative)]
#[display("<Task>")]
pub struct FrozenTask {
    pub(super) r#impl: values::FrozenValue,
    #[allocative(skip)]
    pub(super) args: SmallMap<String, Arg>,
    pub(super) summary: String,
    pub(super) description: String,
    pub(super) display_name: String,
    pub(super) group: Vec<String>,
    pub(super) name: String,
    pub(super) traits: Vec<values::FrozenValue>,
    pub(super) path: PathBuf,
    pub(super) overrides: values::FrozenValue,
}

starlark_simple_value!(FrozenTask);

#[starlark_value(type = "Task")]
impl<'v> StarlarkValue<'v> for FrozenTask {
    type Canonical = Task<'v>;
}

impl FrozenTask {
    pub fn implementation(&self) -> values::FrozenValue {
        self.r#impl
    }
    pub fn args(&self) -> &SmallMap<String, Arg> {
        &self.args
    }
    pub fn traits(&self) -> &[values::FrozenValue] {
        &self.traits
    }
    pub fn path(&self) -> &PathBuf {
        &self.path
    }
    pub fn overrides(&self) -> values::FrozenValue {
        self.overrides
    }
    /// Get trait type IDs this task opts into.
    pub fn trait_type_ids(&self) -> Vec<u64> {
        self.traits
            .iter()
            .filter_map(|f| extract_trait_type_id(f.to_value()))
            .collect()
    }
}

impl<'v> TaskLike<'v> for FrozenTask {
    fn args(&self) -> &SmallMap<String, Arg> {
        &self.args
    }
    fn summary(&self) -> &String {
        &self.summary
    }
    fn description(&self) -> &String {
        &self.description
    }
    fn display_name(&self) -> String {
        self.display_name.clone()
    }
    fn group(&self) -> &Vec<String> {
        &self.group
    }
    fn name(&self) -> String {
        self.name.clone()
    }
    fn path(&self) -> &PathBuf {
        &self.path
    }
    fn implementation(&self) -> Value<'v> {
        self.r#impl.to_value()
    }
    fn trait_type_ids(&self) -> Vec<u64> {
        self.traits
            .iter()
            .filter_map(|f| extract_trait_type_id(f.to_value()))
            .collect()
    }
    fn overrides(&self) -> Value<'v> {
        self.overrides.to_value()
    }
}

struct TaskImpl;

impl StarlarkCallableParamSpec for TaskImpl {
    fn params() -> ParamSpec {
        ParamSpec::new_parts(
            [(ParamIsRequired::Yes, TaskContext::get_type_starlark_repr())],
            [],
            None,
            [],
            None,
        )
        .unwrap()
    }
}

#[starlark_module]
pub fn register_globals(globals: &mut GlobalsBuilder) {
    /// Declares a task — a named CLI command with an implementation function.
    ///
    /// ## Naming
    ///
    /// Assign the result to a **snake_case** variable. The CLI command name is derived
    /// automatically by converting `_` to `-` (`axl_add` → `axl-add`).
    /// Use `name = "explicit-name"` to override.
    /// Command names must match `[a-z][a-z0-9-]*`.
    ///
    /// ## Args
    ///
    /// Arg keys must be `snake_case` (`[a-z][a-z0-9_]*`). There are two kinds:
    ///
    /// - **CLI args** (`args.string(...)`, `args.int(...)`, etc.) — exposed as `--kebab-flags` on
    ///   the CLI and accessible as `ctx.args.arg_name` in the implementation. Can be overridden
    ///   in `config.axl`; an explicit CLI flag always wins over a config override.
    ///
    /// - **Config-only args** (`args.custom(type, default = …)`) — not shown in help; set by repo
    ///   maintainers in `config.axl` via `ctx.tasks["group/name"].args.arg_name = value`.
    ///
    /// All args are read as `ctx.args.arg_name` in the implementation regardless of kind.
    ///
    /// ## Help text
    ///
    /// - `summary` — one-liner shown in the task list; falls back to `"<name> task defined in <file>"`.
    /// - `description` — extended prose shown in `--help` (replaces summary in that view).
    /// - `display_name` — Title Case name for help section headings; auto-derived from command name.
    ///
    /// ## Example
    ///
    /// ```starlark
    /// def _impl(ctx: TaskContext) -> int:
    ///     ctx.std.io.stdout.write("Hello, " + ctx.args.recipient + "\n")
    ///     return 0
    ///
    /// greet = task(
    ///     group = ["utils"],
    ///     summary = "Say hello",
    ///     implementation = _impl,
    ///     args = {
    ///         "recipient": args.string(default = "world", description = "Who to greet"),
    ///         "greeting":  args.custom(str, default = "Hello", description = "Greeting word (config.axl only)"),
    ///     },
    /// )
    /// ```
    fn task<'v>(
        #[starlark(require = named)] implementation: values::typing::StarlarkCallable<
            'v,
            TaskImpl,
            NoneType,
        >,
        #[starlark(require = named, default = values::dict::UnpackDictEntries::default())]
        args: values::dict::UnpackDictEntries<String, Value<'v>>,
        #[starlark(require = named, default = String::new())] summary: String,
        #[starlark(require = named, default = String::new())] description: String,
        #[starlark(require = named, default = String::new())] display_name: String,
        #[starlark(require = named, default = UnpackList::default())] group: UnpackList<String>,
        #[starlark(require = named, default = String::new())] name: String,
        #[starlark(require = named, default = UnpackList::default())] traits: UnpackList<Value<'v>>,
        eval: &mut starlark::eval::Evaluator<'v, '_, '_>,
    ) -> anyhow::Result<Task<'v>> {
        let path = Env::current_script_path(eval)?;
        let overrides = eval.heap().alloc(Arguments::new());
        if group.items.len() > MAX_TASK_GROUPS {
            return Err(anyhow::anyhow!(
                "task cannot have more than {} group levels",
                MAX_TASK_GROUPS
            )
            .into());
        }
        // Validate name if explicitly set (empty means "derive from variable name via to_command_name").
        if !name.is_empty() {
            validate_command_name(&name, "task").map_err(|e| anyhow::anyhow!(e))?;
        }

        // Validate group elements.
        for g in &group.items {
            validate_command_name(g, "group").map_err(|e| anyhow::anyhow!(e))?;
        }

        let display_name = if !display_name.is_empty() {
            display_name
        } else if !name.is_empty() {
            to_display_name(&name)
        } else {
            String::new()
        };

        // Parse and validate args.
        let mut args_ = SmallMap::new();
        for (arg_name, value) in args.entries {
            validate_arg_name(&arg_name).map_err(|e| anyhow::anyhow!("task {}", e))?;
            let cli_arg = value.downcast_ref::<Arg>().ok_or_else(|| {
                anyhow::anyhow!(
                    "task arg {:?}: expected args.string/boolean/int/uint/... or args.custom(...), got '{}'",
                    arg_name,
                    value.get_type()
                )
            })?.clone();
            if let Some(lo) = cli_arg.long_override() {
                if lo.contains(':') {
                    return Err(anyhow::anyhow!(
                        "task arg {:?}: `long` override may not contain ':'; \
                         namespaced overrides (e.g. \"feature:flag\") are only valid for feature args",
                        arg_name
                    ).into());
                }
            }
            args_.insert(arg_name, cli_arg);
        }

        // Validate each element is a TraitType or FrozenTraitType.
        let all_traits = traits.items;
        for t in &all_traits {
            if t.downcast_ref::<TraitType>().is_none()
                && t.downcast_ref::<FrozenTraitType>().is_none()
            {
                return Err(anyhow::anyhow!(
                    "traits list must contain trait types, got '{}'",
                    t.get_type()
                )
                .into());
            }
        }

        Ok(Task {
            args: args_,
            r#impl: implementation.0,
            summary,
            description,
            display_name: RefCell::new(display_name),
            group: group.items,
            name: RefCell::new(name),
            traits: all_traits,
            path,
            overrides,
        })
    }
}
