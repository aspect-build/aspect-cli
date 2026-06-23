use std::cell::RefCell;
use std::path::PathBuf;

use allocative::Allocative;
use derive_more::Display;
use starlark::collections::SmallMap;
use starlark::environment::GlobalsBuilder;
use starlark::environment::Methods;
use starlark::environment::MethodsBuilder;
use starlark::environment::MethodsStatic;
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
use starlark::values::dict::UnpackDictEntries;
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
    /// The task kind's display label (Title-Cased kind, or the `friendly_kind=`
    /// kwarg). Distinct from the per-invocation display name set at runtime.
    fn friendly_kind(&self) -> String;
    fn group(&self) -> &Vec<String>;
    /// The task kind — the command being run (e.g. `build`, `test`, `lint`),
    /// derived from the snake_case export variable (or the `kind=` kwarg).
    fn kind(&self) -> String;
    /// Absolute path to the .axl file the task was defined in.
    fn path(&self) -> &PathBuf;
    /// `Arguments` value carrying config.axl overrides for this task.
    /// Live tasks return their mutable store directly; frozen tasks return
    /// the frozen variant lifted to the live heap via `to_value()`.
    fn overrides(&self) -> Value<'v>;
    /// Implementation function lifted to the live heap via `to_value()`.
    fn implementation(&self) -> Value<'v>;
    /// Trait values this task opts into, lifted to the live heap via
    /// `to_value()`. The returned `Vec` is suitable for storing on a fresh
    /// `Task<'v>` (e.g. when building an alias).
    fn trait_values(&self) -> Vec<Value<'v>>;

    /// Trait type ids this task opts into (via the `traits = [...]` kwarg).
    fn trait_type_ids(&self) -> Vec<u64> {
        self.trait_values()
            .into_iter()
            .filter_map(extract_trait_type_id)
            .collect()
    }

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
/// `TaskMap` entries without caring whether they are live or frozen. The
/// `Value<'v>` impl panics on type mismatch — use `try_as_task` when the
/// value's type is not statically guaranteed.
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
        try_as_task(*self).unwrap_or_else(|| {
            panic!("expected a task value, got '{}'", self.get_type());
        })
    }
}

/// Non-panicking variant of `AsTaskLike::as_task`. Returns `None` when the
/// value is neither a `Task` nor a `FrozenTask`.
pub fn try_as_task<'v>(value: Value<'v>) -> Option<&'v dyn TaskLike<'v>> {
    if let Some(t) = value.downcast_ref::<Task<'v>>() {
        return Some(t);
    }
    if let Some(t) = value.downcast_ref::<FrozenTask>() {
        return Some(t);
    }
    None
}

#[derive(Debug, Clone, ProvidesStaticType, Display, Trace, NoSerialize, Allocative)]
#[display("<Task>")]
pub struct Task<'v> {
    r#impl: values::Value<'v>,
    #[allocative(skip)]
    pub(super) args: SmallMap<String, Arg>,
    pub(super) summary: String,
    pub(super) description: String,
    pub(super) friendly_kind: RefCell<String>,
    pub(super) group: Vec<String>,
    pub(super) kind: RefCell<String>,
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
    pub fn friendly_kind(&self) -> String {
        self.friendly_kind.borrow().clone()
    }
    pub fn group(&self) -> &Vec<String> {
        &self.group
    }
    pub fn kind(&self) -> String {
        self.kind.borrow().clone()
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
        let overrides = heap.alloc(Arguments::with_schema(frozen.args.keys().cloned()));
        let traits: Vec<Value<'v>> = frozen.traits.iter().map(|fv| fv.to_value()).collect();
        Task {
            r#impl: frozen.r#impl.to_value(),
            args: frozen.args.clone(),
            summary: frozen.summary.clone(),
            description: frozen.description.clone(),
            friendly_kind: RefCell::new(frozen.friendly_kind.clone()),
            group: frozen.group.clone(),
            kind: RefCell::new(frozen.kind.clone()),
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
    fn friendly_kind(&self) -> String {
        self.friendly_kind.borrow().clone()
    }
    fn group(&self) -> &Vec<String> {
        &self.group
    }
    fn kind(&self) -> String {
        self.kind.borrow().clone()
    }
    fn path(&self) -> &PathBuf {
        &self.path
    }
    fn implementation(&self) -> Value<'v> {
        self.r#impl
    }
    fn trait_values(&self) -> Vec<Value<'v>> {
        self.traits.clone()
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
        let mut kind = self.kind.borrow_mut();
        if kind.is_empty() {
            *kind = kebab;
        }
        let mut friendly_kind = self.friendly_kind.borrow_mut();
        if friendly_kind.is_empty() {
            *friendly_kind = to_display_name(&kind);
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

    fn get_methods() -> Option<&'static Methods> {
        static RES: MethodsStatic = MethodsStatic::new();
        RES.methods(task_methods)
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
            friendly_kind: self.friendly_kind.into_inner(),
            group: self.group,
            kind: self.kind.into_inner(),
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
    pub(super) friendly_kind: String,
    pub(super) group: Vec<String>,
    pub(super) kind: String,
    pub(super) traits: Vec<values::FrozenValue>,
    pub(super) path: PathBuf,
    pub(super) overrides: values::FrozenValue,
}

starlark_simple_value!(FrozenTask);

#[starlark_value(type = "Task")]
impl<'v> StarlarkValue<'v> for FrozenTask {
    type Canonical = Task<'v>;

    fn get_methods() -> Option<&'static Methods> {
        static RES: MethodsStatic = MethodsStatic::new();
        RES.methods(task_methods)
    }
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
    fn friendly_kind(&self) -> String {
        self.friendly_kind.clone()
    }
    fn group(&self) -> &Vec<String> {
        &self.group
    }
    fn kind(&self) -> String {
        self.kind.clone()
    }
    fn path(&self) -> &PathBuf {
        &self.path
    }
    fn implementation(&self) -> Value<'v> {
        self.r#impl.to_value()
    }
    fn trait_values(&self) -> Vec<Value<'v>> {
        self.traits.iter().map(|f| f.to_value()).collect()
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

/// Validate `kind`/`group` and resolve `friendly_kind` per the rules shared by
/// `task()` and `task.alias(...)`.
///
/// - `group` length is capped at `MAX_TASK_GROUPS`.
/// - `kind`, when non-empty, must match `[a-z][a-z0-9-]*`. An empty `kind`
///   defers naming to `export_as` (which fills from the variable name).
/// - Each `group` element must match the same pattern as `kind`.
/// - The returned `friendly_kind` is `friendly_kind` verbatim if non-empty,
///   then a Title-Case derivation of `kind` if `kind` is non-empty, else
///   empty (deferred until `export_as` populates `kind`).
///
/// `context` is the user-facing identifier used in error messages — typically
/// `"task"` or `"task.alias"`.
fn resolve_task_metadata(
    context: &str,
    kind: &str,
    group: &[String],
    friendly_kind: String,
) -> anyhow::Result<String> {
    if group.len() > MAX_TASK_GROUPS {
        return Err(anyhow::anyhow!(
            "{} cannot have more than {} group levels",
            context,
            MAX_TASK_GROUPS,
        ));
    }
    if !kind.is_empty() {
        validate_command_name(kind, context).map_err(|e| anyhow::anyhow!(e))?;
    }
    for g in group {
        validate_command_name(g, "group").map_err(|e| anyhow::anyhow!(e))?;
    }
    Ok(if !friendly_kind.is_empty() {
        friendly_kind
    } else if !kind.is_empty() {
        to_display_name(kind)
    } else {
        String::new()
    })
}

/// Build a fresh `Task<'v>` that aliases `base`. The alias shares the base's
/// `implementation` callable and `traits` vector and inherits nothing else —
/// `kind`, `group`, `summary`, `description`, and `friendly_kind` come from
/// the alias's own kwargs. An empty `kind` defers naming to `export_as`.
///
/// `defaults` may overlay new defaults onto any arg present on `base`; see
/// `Arg::with_default` for the per-variant validation rules.
fn build_alias<'v>(
    base: &dyn TaskLike<'v>,
    defaults: UnpackDictEntries<String, Value<'v>>,
    summary: String,
    description: String,
    friendly_kind: String,
    group: Vec<String>,
    kind: String,
    eval: &mut starlark::eval::Evaluator<'v, '_, '_>,
) -> anyhow::Result<Task<'v>> {
    let friendly_kind = resolve_task_metadata("task.alias", &kind, &group, friendly_kind)?;

    let mut overlaid = base.args().clone();
    for (k, v) in defaults.entries {
        let Some(existing) = overlaid.get(&k) else {
            return Err(anyhow::anyhow!(
                "task.alias defaults[{:?}]: no such arg on aliased task `{}`",
                k,
                base.kind(),
            ));
        };
        let next = existing.with_default(&k, v, eval.heap())?;
        overlaid.insert(k, next);
    }

    // `(alias of `<base>`)` is appended to `description` so it shows up in
    // `<task> --help`. The compact `aspect --help` task list still reads from
    // `summary` and stays uncluttered. When the user supplied a `summary` but
    // no `description`, seed the description from the summary so the user's
    // text isn't displaced by the hint in the per-task help view.
    let alias_hint = format!("(alias of `{}`)", base.kind());
    let description = match (description.is_empty(), summary.is_empty()) {
        (true, true) => alias_hint,
        (true, false) => format!("{}\n\n{}", summary, alias_hint),
        (false, _) => format!("{}\n\n{}", description, alias_hint),
    };

    let overrides = eval
        .heap()
        .alloc(Arguments::with_schema(overlaid.keys().cloned()));
    Ok(Task {
        args: overlaid,
        r#impl: base.implementation(),
        summary,
        description,
        friendly_kind: RefCell::new(friendly_kind),
        group,
        kind: RefCell::new(kind),
        traits: base.trait_values(),
        path: Env::current_script_path(eval)?,
        overrides,
    })
}

#[starlark_module]
fn task_methods(builder: &mut MethodsBuilder) {
    /// Define an alias of this task with overridden arg defaults.
    ///
    /// The alias is a new top-level CLI command that shares this task's
    /// `implementation` and `traits`, but exposes overridden defaults for one
    /// or more of its args. The base task is undisturbed — both commands
    /// coexist, and a CLI flag still wins over the alias's default (so
    /// `aspect buildifier --formatter-target=X` overrides the alias default).
    ///
    /// ## Naming
    ///
    /// Assign the result to a **snake_case** variable; the task kind (CLI
    /// command name) is derived from the variable. The base task's kind is
    /// never inherited. Use `kind = "explicit-kind"` to override.
    ///
    /// ## Constraints
    ///
    /// - `defaults` keys must already exist on the base task.
    /// - The value type must match the base arg's variant (`args.string` →
    ///   `str`, `args.int` → `int`, `args.string_list` → `list[str]`, etc.).
    ///   `args.string(values = [...])` re-checks membership on the new default.
    /// - `args.trailing_var_args` carries no default in the schema and cannot
    ///   be overridden via `defaults`.
    /// - Aliases cannot add args beyond the base. Use `task()` if you need to.
    ///
    /// ## Example
    ///
    /// ```starlark
    /// load("@aspect//format.axl", "format")
    ///
    /// buildifier = format.alias(
    ///     defaults = {
    ///         "formatter_target": "@buildifier_prebuilt//buildifier",
    ///     },
    ///     summary = "Format Starlark files with buildifier.",
    /// )
    ///
    /// def config(ctx: ConfigContext):
    ///     ctx.tasks.add(buildifier)
    /// ```
    fn alias<'v>(
        this: Value<'v>,
        #[starlark(require = named, default = UnpackDictEntries::default())]
        defaults: UnpackDictEntries<String, Value<'v>>,
        #[starlark(require = named, default = String::new())] summary: String,
        #[starlark(require = named, default = String::new())] description: String,
        #[starlark(require = named, default = String::new())] friendly_kind: String,
        #[starlark(require = named, default = UnpackList::default())] group: UnpackList<String>,
        #[starlark(require = named, default = String::new())] kind: String,
        eval: &mut starlark::eval::Evaluator<'v, '_, '_>,
    ) -> anyhow::Result<Task<'v>> {
        let base = try_as_task(this).ok_or_else(|| {
            anyhow::anyhow!(
                "task.alias: expected a Task value, got '{}'",
                this.get_type(),
            )
        })?;
        build_alias(
            base,
            defaults,
            summary,
            description,
            friendly_kind,
            group.items,
            kind,
            eval,
        )
    }
}

#[starlark_module]
pub fn register_globals(globals: &mut GlobalsBuilder) {
    /// Declares a task — a named CLI command with an implementation function.
    ///
    /// ## Naming
    ///
    /// Assign the result to a **snake_case** variable. The task *kind* (the CLI
    /// command name) is derived automatically by converting `_` to `-`
    /// (`axl_add` → `axl-add`). Use `kind = "explicit-kind"` to override it.
    /// Kinds must match `[a-z][a-z0-9-]*`. (The per-invocation task *name* — the
    /// unique identity of one run — is a separate concept set at runtime via
    /// `--task:name`; see `ctx.task`.)
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
    /// - `friendly_kind` — Title Case label for help section headings; auto-derived from the kind.
    ///
    /// ## Aliases
    ///
    /// Call `<task>.alias(defaults = {...})` to declare a new top-level command
    /// that shares this task's implementation but exposes overridden defaults
    /// for one or more args. See the `task.alias` docstring for details.
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
        #[starlark(require = named, default = String::new())] friendly_kind: String,
        #[starlark(require = named, default = UnpackList::default())] group: UnpackList<String>,
        #[starlark(require = named, default = String::new())] kind: String,
        #[starlark(require = named, default = UnpackList::default())] traits: UnpackList<Value<'v>>,
        eval: &mut starlark::eval::Evaluator<'v, '_, '_>,
    ) -> anyhow::Result<Task<'v>> {
        let friendly_kind = resolve_task_metadata("task", &kind, &group.items, friendly_kind)?;

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
            if let Some(lo) = cli_arg.long_override()
                && lo.contains(':')
            {
                return Err(anyhow::anyhow!(
                    "task arg {:?}: `long` override may not contain ':'; \
                     namespaced overrides (e.g. \"feature:flag\") are only valid for feature args",
                    arg_name,
                ));
            }
            args_.insert(arg_name, cli_arg);
        }

        // Trait list elements must be TraitType / FrozenTraitType values.
        let all_traits = traits.items;
        for t in &all_traits {
            if t.downcast_ref::<TraitType>().is_none()
                && t.downcast_ref::<FrozenTraitType>().is_none()
            {
                return Err(anyhow::anyhow!(
                    "traits list must contain trait types, got '{}'",
                    t.get_type()
                ));
            }
        }

        let overrides = eval
            .heap()
            .alloc(Arguments::with_schema(args_.keys().cloned()));
        Ok(Task {
            args: args_,
            r#impl: implementation.0,
            summary,
            description,
            friendly_kind: RefCell::new(friendly_kind),
            group: group.items,
            kind: RefCell::new(kind),
            traits: all_traits,
            path: Env::current_script_path(eval)?,
            overrides,
        })
    }
}

#[cfg(test)]
mod tests {
    //! Tests for `task.alias(...)` and the shared `resolve_task_metadata`
    //! helper. Behavior pinned here: the alias's impl runs (inheritance),
    //! arg-default overlays succeed across all variants, schema constraints
    //! on the base survive the overlay, and validation errors surface
    //! useful messages.

    /// Boilerplate every alias test reuses: a no-op `_impl` plus a `base`
    /// task that the test extends or aliases.
    const BASE_PRELUDE: &str = r#"
def _impl(ctx):
    return 0
"#;

    fn eval_snippet(extra: &str) -> crate::test::EvalBuilder {
        crate::test::eval(&format!("{BASE_PRELUDE}{extra}"))
    }

    fn assert_eval_err_contains(extra: &str, needle: &str) {
        let err = eval_snippet(extra).check().expect_err("expected error");
        let msg = err.to_string();
        assert!(msg.contains(needle), "expected {needle:?} in {msg:?}");
    }

    #[test]
    fn alias_inherits_implementation() {
        let exit = eval_snippet(
            r#"
base = task(implementation = _impl)
aliased = base.alias()
"#,
        )
        .run_task(1)
        .expect("run_task");
        assert_eq!(exit, Some(0));
    }

    #[test]
    fn alias_of_alias_runs() {
        let exit = eval_snippet(
            r#"
base = task(implementation = _impl)
first = base.alias()
second = first.alias()
"#,
        )
        .run_task(2)
        .expect("run_task");
        assert_eq!(exit, Some(0));
    }

    #[test]
    fn alias_unknown_default_key_errors() {
        assert_eval_err_contains(
            r#"
base = task(
    implementation = _impl,
    args = {"mode": args.string(default = "auto")},
)
buildifier = base.alias(defaults = {"nope": "x"})
"#,
            "no such arg",
        );
    }

    #[test]
    fn alias_scalar_defaults_overlaid() {
        eval_snippet(
            r#"
base = task(
    implementation = _impl,
    args = {
        "name":    args.string(default = ""),
        "verbose": args.boolean(default = False),
        "retries": args.int(default = 0),
        "port":    args.uint(default = 0),
    },
)
aliased = base.alias(defaults = {
    "name":    "preset",
    "verbose": True,
    "retries": 3,
    "port":    8080,
})
"#,
        )
        .check()
        .expect("alias with scalar defaults should evaluate");
    }

    #[test]
    fn alias_list_defaults_overlaid() {
        eval_snippet(
            r#"
base = task(
    implementation = _impl,
    args = {
        "tags":     args.string_list(default = []),
        "depths":   args.int_list(default = []),
        "ports":    args.uint_list(default = []),
        "flags":    args.boolean_list(default = []),
        "files":    args.positional(default = []),
    },
)
aliased = base.alias(defaults = {
    "tags":   ["a", "b"],
    "depths": [1, 2, 3],
    "ports":  [80, 443],
    "flags":  [True, False],
    "files":  ["x.txt", "y.txt"],
})
"#,
        )
        .check()
        .expect("alias with list defaults should evaluate");
    }

    #[test]
    fn alias_type_mismatch_errors() {
        assert_eval_err_contains(
            r#"
base = task(
    implementation = _impl,
    args = {"mode": args.string(default = "auto")},
)
buildifier = base.alias(defaults = {"mode": 42})
"#,
            "expected string",
        );
    }

    #[test]
    fn alias_list_element_type_mismatch_errors() {
        assert_eval_err_contains(
            r#"
base = task(
    implementation = _impl,
    args = {"tags": args.string_list(default = [])},
)
buildifier = base.alias(defaults = {"tags": [1, 2, 3]})
"#,
            "expected string_list",
        );
    }

    #[test]
    fn alias_string_values_constraint_enforced() {
        assert_eval_err_contains(
            r#"
base = task(
    implementation = _impl,
    args = {"mode": args.string(default = "auto", values = ["auto", "fail"])},
)
buildifier = base.alias(defaults = {"mode": "nope"})
"#,
            "not one of the allowed values",
        );
    }

    #[test]
    fn alias_trailing_var_args_default_rejected() {
        assert_eval_err_contains(
            r#"
base = task(
    implementation = _impl,
    args = {"tail": args.trailing_var_args()},
)
buildifier = base.alias(defaults = {"tail": []})
"#,
            "trailing_var_args",
        );
    }

    #[test]
    fn alias_custom_default_type_validated() {
        assert_eval_err_contains(
            r#"
base = task(
    implementation = _impl,
    args = {"opt": args.custom(str, default = "x")},
)
buildifier = base.alias(defaults = {"opt": 42})
"#,
            "does not match arg type",
        );
    }

    #[test]
    fn alias_invalid_kind_rejected() {
        assert_eval_err_contains(
            r#"
base = task(implementation = _impl)
aliased = base.alias(kind = "Bad-Kind")
"#,
            "task.alias",
        );
    }

    #[test]
    fn alias_too_many_group_levels_rejected() {
        assert_eval_err_contains(
            r#"
base = task(implementation = _impl)
aliased = base.alias(group = ["a", "b", "c", "d", "e", "f"])
"#,
            "group levels",
        );
    }
}
