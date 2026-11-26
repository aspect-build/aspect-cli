use crate::engine::history::TaskStart as HistoryTaskStart;
use crate::errln;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use anyhow::anyhow;
use starlark::environment::{FrozenModule, Module};
use starlark::eval::Evaluator;
use starlark::values::{Heap, Value, ValueLike};
use uuid::Uuid;

use crate::banner;
use crate::ci::on_recognized_ci;
use crate::console::{Console, Host, Slot, TaskIdentity};
use crate::diag;
use crate::engine::arguments::Arguments;
use crate::engine::bazel::{Bazel, ClaimCheck};
use crate::engine::cancellation::Signals;
use crate::engine::config_context::ConfigContext;
use crate::engine::console as console_value;
use crate::engine::feature::{Feature, FeatureLike, FrozenFeature};
use crate::engine::feature_context::FeatureContext;
use crate::engine::feature_map::FeatureMap;
use crate::engine::passthrough;
use crate::engine::task::{FrozenTask, Task, TaskLike};
use crate::engine::task_context::TaskContext;
use crate::engine::task_hooks::{self, TaskHooks};
use crate::engine::task_info::PhaseRecord;
use crate::engine::task_info::TaskInfo;
use crate::engine::task_map::TaskMap;
use crate::engine::telemetry::{self, ExporterSpec, Telemetry};
use crate::engine::r#trait::{TraitInstance, extract_trait_type_id};
use crate::engine::trait_map::TraitMap;
use crate::eval::error::EvalError;
use crate::eval::exit::TaskExit;
use crate::eval::load::AxlLoader;
use crate::eval::outcome::{Ending, Outcome};
use crate::eval::task::FrozenTaskModuleLike;
use crate::module::Mod;

/// `(open, reset)` for `role` of `palette` on a console line, both empty when
/// color is off or the role has no styling. The opening "Running" line takes
/// `highlight`, as AXL's phase lines do, and the closing verdict
/// `label.success`, `label.warning` or `label.error`. `color` is
/// [`crate::console::Console::color`].
fn role_style(palette: &aspect_theme::Palette, role: &str, color: bool) -> (String, &'static str) {
    let open = if color {
        palette.sgr(role)
    } else {
        String::new()
    };
    let reset = if open.is_empty() {
        ""
    } else {
        aspect_theme::RESET
    };
    (open, reset)
}

/// The task label for the runtime's opening "Running …" and closing
/// "Passed/Failed …" bookend lines. Reads the CI environment; see
/// [`task_label_for`] for the pure decision.
fn task_label(group: &[String], kind: &str, name: &str, name_meaningful: bool) -> String {
    task_label_for(group, kind, name, name_meaningful, on_recognized_ci())
}

/// The command path shown to the user: the task's group(s) and kind joined by
/// spaces, matching the CLI invocation (`aspect auth configure`). A top-level
/// task (no group) is just its kind (`build`).
fn task_command_path(group: &[String], kind: &str) -> String {
    if group.is_empty() {
        kind.to_string()
    } else {
        format!("{} {}", group.join(" "), kind)
    }
}

/// CLI prefix of the builtin feature whose args select the terminal surface.
const CONSOLE_FEATURE: &str = "console";

/// Pure core of [`task_label`], env-injected for testing.
///
/// The task is identified by its command path (`<group…> <kind>`, space-joined
/// to match what the user typed — `auth configure`). When the name carries
/// information beyond the kind, mirror the CI status surfaces: lead with the name
/// and bracket the path as extra context (`<name> [<path>]`). That's the case
/// when the name differs from the kind AND either it's meaningful (an explicit
/// `--task:name` / CI-job-derived name) or we're on CI (where the name is part of
/// the status-check identity). For a bare local run the name is a
/// `<kind>-<random-suffix>` placeholder that reads as noise, so show just
/// `<path> task`.
fn task_label_for(
    group: &[String],
    kind: &str,
    name: &str,
    name_meaningful: bool,
    on_ci: bool,
) -> String {
    let path = task_command_path(group, kind);
    if name != kind && (name_meaningful || on_ci) {
        format!("{name} [{path}]")
    } else {
        format!("{path} task")
    }
}

/// What a startup phase was loading an AXL file for, and so what its failure
/// costs. Carried on [`AxlFileFailure`] because the phase is the only thing
/// that knows: the file itself looks the same whichever way it was reached.
///
/// The wording shown to the user is the caller's — `aspect-cli`'s `main` turns
/// a role into the clause its `warning:` line ends with.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AxlFileRole {
    /// A discovered `.aspect/*.axl` script, or a `use_task(...)` reference.
    /// Either way only tasks are registered out of it.
    Tasks,
    /// A `use_feature(...)` reference.
    Feature,
    /// One file reached both ways, so its absence costs both.
    TasksAndFeature,
    /// A `config.axl`.
    Config,
}

impl AxlFileRole {
    /// The role of a file reached two ways, both of which its failure costs.
    ///
    /// Collapsing every disagreement to [`Self::TasksAndFeature`] is exact
    /// because that is the only disagreement reachable: phase 1 records
    /// [`Self::Tasks`] and [`Self::Feature`], phase 2 only [`Self::Config`],
    /// and each phase accumulates into its own list — so `Config` never meets
    /// either of the others, however many modules name the same path.
    fn merged(self, other: Self) -> Self {
        if self == other {
            self
        } else {
            Self::TasksAndFeature
        }
    }
}

/// An AXL file that failed during startup evaluation, recorded instead of
/// raised.
///
/// Phases 1 and 2 collect these so one unloadable file costs only what that
/// file defines. Whoever drove the phase decides what a non-empty list means,
/// but ignoring a non-empty one hides a real error — it is never "empty and
/// fine".
///
/// A [`TaskExit`] is deliberately *not* one of these; see
/// [`MultiPhaseEval::ends_the_run`].
#[derive(Debug)]
pub struct AxlFileFailure {
    /// Absolute path of the `.axl` file that failed.
    pub path: PathBuf,
    /// What the file was being loaded for when it failed.
    pub role: AxlFileRole,
    pub error: EvalError,
}

/// What a tolerant startup phase came back with.
///
/// The two fields travel together because both have to be reported. `ended`
/// is what the run must end on whatever the command; `files` is what the
/// phase had already recorded on the way to it, and a typo recorded before a
/// `ctx.std.process.exit()` gate fired is still a typo the user needs to
/// hear about. Raising `ended` on its own would drop those, which is the
/// silence this whole tolerance exists to avoid.
#[derive(Debug, Default)]
pub struct PhaseFailures {
    /// Files that failed without ending the run.
    pub files: Vec<AxlFileFailure>,
    /// The error that ended the phase early, if one did. Nothing after it in
    /// the phase ran, so `files` is only what came before.
    pub ended: Option<EvalError>,
}

impl PhaseFailures {
    /// Whether the phase came back with nothing to report.
    pub fn is_empty(&self) -> bool {
        self.files.is_empty() && self.ended.is_none()
    }
}

/// Wrapper around a live Starlark Module heap.
///
/// All three evaluation phases share this heap so `Value<'v>` references
/// are valid across phase boundaries without freeze/thaw cycles.
pub struct ModuleEnv<'v>(pub(crate) Module<'v>);

impl<'v> ModuleEnv<'v> {
    /// Run a closure with a fresh module heap, hiding the heap lifetime from the caller.
    pub fn with<R, E>(func: impl for<'a> FnOnce(&ModuleEnv<'a>) -> Result<R, E>) -> Result<R, E> {
        Module::with_temp_heap(|m| func(&ModuleEnv(m)))
    }

    pub fn heap(&self) -> Heap<'v> {
        self.0.heap()
    }
}

/// Multi-phase Starlark evaluator with one shared Module heap.
///
/// Phases:
/// 1. `eval`                        — load task and feature scripts; inserts
///                                    discovered `Task` and `Feature` values
///                                    into `self.tasks` / `self.features` on
///                                    the shared heap.
/// 2. `execute_configs`             — run config.axl files; mutations visible
///                                    via shared heap.
/// 3. `execute_features_with_args`  — run enabled feature implementations.
/// 4. `execute_tasks_with_args`     — call the selected task implementation.
///
/// Phases 1 and 2 report an unloadable file as an [`AxlFileFailure`] rather
/// than stopping, so one bad `.aspect/*.axl` costs only what that file
/// defines; every later phase still fails outright. Phase 2 therefore runs
/// even when Phase 1 recorded a failure — the tolerance is per file, not per
/// phase.
pub struct MultiPhaseEval<'v, 'l> {
    env: &'l ModuleEnv<'v>,
    loader: &'l AxlLoader<'l>,
    /// Global task map on the shared heap. Eagerly allocated empty in `new()`;
    /// Phase 1 inserts each discovered task; `ctx.tasks.add(...)` may insert more
    /// during Phase 2.
    tasks: Value<'v>,
    /// Global feature map on the shared heap. Eagerly allocated empty in `new()`;
    /// Phase 1 inserts each discovered feature.
    features: Value<'v>,
    /// The run's one trait map on the shared heap, eagerly allocated empty
    /// in `new()`: `ctx.traits` for configs, features and the task. Any
    /// trait type's instance is created in it on first access.
    trait_map_value: Value<'v>,
    /// Telemetry handle allocated on the heap and shared between
    /// ConfigContext and FeatureContext as `ctx.telemetry`. The runtime
    /// drains exporter specs out of it (via `drain_exporters`) after phase 3.
    telemetry_value: Value<'v>,
    /// The run's `TaskHooks`, shared by ConfigContext, FeatureContext, and
    /// TaskContext as `ctx.hooks`; phase 4 runs what was registered into it.
    hooks_value: Value<'v>,
}

impl<'v, 'l> MultiPhaseEval<'v, 'l> {
    pub fn new(env: &'l ModuleEnv<'v>, loader: &'l AxlLoader<'l>) -> Self {
        let heap = env.heap();
        MultiPhaseEval {
            env,
            loader,
            tasks: heap.alloc(TaskMap::new()),
            features: heap.alloc(FeatureMap::new()),
            trait_map_value: heap.alloc(TraitMap::new()),
            telemetry_value: Telemetry::alloc(heap, loader.env.telemetry.clone()),
            hooks_value: TaskHooks::alloc(heap),
        }
    }

    /// Drain exporter specs collected from `ctx.telemetry.exporters.add(...)`
    /// during phases 2-3. Intended to be called once, after
    /// `execute_features_with_args` completes and before phase 4 begins.
    pub fn drain_exporters(&self) -> Vec<ExporterSpec> {
        let specs = telemetry::drain_exporters(self.telemetry_value);
        self.loader.env.telemetry.set_exported(&specs);
        specs
    }

    fn heap(&self) -> Heap<'v> {
        self.env.0.heap()
    }

    /// Load and evaluate an AXL file via AxlLoader (per-file frozen module).
    /// The loader manages caching, cycle detection, and the module-scope stack.
    #[tracing::instrument(level = "debug", skip_all, fields(path = %abs_path.display()))]
    fn eval_file(&self, scope: &'l Mod, abs_path: &Path) -> Result<FrozenModule, EvalError> {
        self.loader.eval_module(scope, abs_path)
    }

    /// Phase 1: evaluate task and feature scripts plus module specs.
    ///
    /// Each file is loaded into its own frozen module (via `AxlLoader::eval_module`).
    /// `FrozenTask` and `FrozenFeature` values discovered in those modules are
    /// inserted into `self.tasks` and `self.features` on the shared heap.
    /// Files are read and parsed ahead of their evaluation on worker threads
    /// (see `eval::prefetch`); evaluation itself stays on this thread.
    /// Phase 2 does not do this: a config can write a file a later one
    /// loads.
    ///
    /// A file that cannot be loaded does not stop the phase: it comes back as
    /// an [`AxlFileFailure`] and only the tasks and features *it* declares are
    /// missing, so the rest of the surface is still discovered. That is what
    /// keeps a typo in one `.aspect/*.axl` file from costing the user every
    /// `aspect` command, `aspect help` included. A file registers its tasks
    /// only once every symbol it was asked for has checked out, so a file that
    /// fails contributes nothing to the shared heap — not even the symbols
    /// that were fine.
    ///
    /// An error that must end the run whatever the command comes back as
    /// [`PhaseFailures::ended`] — see [`Self::ends_the_run`]. It does not cut
    /// this phase short: every script is still loaded, so one pass reports
    /// every typo and the output does not depend on `read_dir` order.
    ///
    /// A caller that needs the whole surface must treat a non-empty return as
    /// the failure it is — `aspect-cli`'s `main` holds the policy for which
    /// commands may run degraded, and [`crate::test::EvalBuilder`] raises the
    /// first failure so an AXL test still fails on a bad snippet.
    #[tracing::instrument(skip_all)]
    pub fn eval(
        &mut self,
        scripts: &[PathBuf],
        root_mod: &'l Mod,
        modules: &'l Vec<Mod>,
    ) -> PhaseFailures {
        let mut out = PhaseFailures::default();

        let roots = scripts
            .iter()
            .map(|path| (root_mod, path.clone()))
            .chain(modules.iter().chain([root_mod]).flat_map(|r#mod| {
                let tasks = r#mod.tasks.keys().map(move |path| (r#mod, path.clone()));
                let features = r#mod
                    .features
                    .iter()
                    .map(move |(path, _)| (r#mod, path.clone()));
                tasks.chain(features)
            }))
            .collect();
        let loader = self.loader;
        loader.with_prefetch(roots, || {
            // Evaluate auto-discovered AXL scripts (axl_sources in repo root)
            for path in scripts {
                if let Err(error) = self.eval_script_tasks(root_mod, path) {
                    self.record(&mut out, path, AxlFileRole::Tasks, error);
                }
            }

            // Evaluate module specs (use_task and use_feature entries from MODULE.aspect)
            for mode in modules.iter().chain(vec![root_mod]) {
                for (abs_path, (label, symbols)) in mode.tasks.iter() {
                    if let Err(error) = self.eval_used_tasks(mode, abs_path, label, symbols) {
                        self.record(&mut out, abs_path, AxlFileRole::Tasks, error);
                    }
                }

                for (abs_path, symbol) in mode.features.iter() {
                    if let Err(error) = self.eval_used_feature(mode, abs_path, symbol) {
                        self.record(&mut out, abs_path, AxlFileRole::Feature, error);
                    }
                }
            }
        });

        out
    }

    /// Record one file's failure: an error that [`Self::ends_the_run`] goes to
    /// [`PhaseFailures::ended`], first one winning, and anything else onto the
    /// file list under the `role` it was being loaded for.
    ///
    /// The file list holds one entry per path and diagnostic, not one per
    /// reference. A `MODULE.aspect` may name the same file twice — `use_task`
    /// and `use_feature` on one path — and a file that does not parse fails
    /// identically each time; the user needs to hear that once, with both
    /// roles folded into what its absence costs. A *different* diagnostic for
    /// the same path is a second thing to fix and is kept.
    ///
    /// Returns whether an ending error has now been seen, which is Phase 2's
    /// cue to stop. The two phases answer that differently on purpose. Phase 1
    /// keeps loading: the files it has yet to read may hold typos of their own,
    /// reporting them all in one pass is the point of the tolerance, and
    /// stopping early would make the output depend on directory order, since
    /// scripts are discovered by `read_dir`. Phase 2 stops: a config body has
    /// a `ctx` and can write files and send requests, and running more of them
    /// after an author's `exit()` would be doing the work that exit exists to
    /// prevent.
    fn record(
        &self,
        out: &mut PhaseFailures,
        path: &Path,
        role: AxlFileRole,
        error: EvalError,
    ) -> bool {
        if self.ends_the_run(&error) {
            out.ended.get_or_insert(error);
        } else if let Some(seen) = out
            .files
            .iter_mut()
            .find(|f| f.path == path && f.error.to_string() == error.to_string())
        {
            seen.role = seen.role.merged(role);
        } else {
            out.files.push(AxlFileFailure {
                path: path.to_path_buf(),
                role,
                error,
            });
        }
        out.ended.is_some()
    }

    /// Whether `error` is somebody's decision to end the run rather than a
    /// file that would not load: `ctx.std.process.exit(code, message)`, an
    /// error value whose type was declared `traceback = False`, or a wait cut
    /// short by Ctrl-C.
    ///
    /// Such an error carries the code and message the run is meant to end
    /// with, so it keeps propagating past the per-file tolerance. Recording it
    /// instead would turn a deliberate gate — a `config.axl` that exits with
    /// "this repo requires a newer aspect CLI" — into a warning on exactly the
    /// commands someone runs to check their setup, and would print the
    /// traceback that [`TaskExit`] exists to suppress.
    fn ends_the_run(&self, error: &EvalError) -> bool {
        TaskExit::from_eval_error(error, &self.loader.env.signals).is_some()
    }

    /// Register every task a discovered script exports. One unit of Phase 1's
    /// per-file fault tolerance — see [`Self::eval`].
    fn eval_script_tasks(&self, scope: &'l Mod, path: &Path) -> Result<(), EvalError> {
        let frozen = self.eval_file(scope, path)?;
        let symbols = frozen.tasks();
        self.register_tasks(&frozen, symbols.iter().map(String::as_str))
    }

    /// Register the tasks one `use_task(label, symbols)` entry names.
    fn eval_used_tasks(
        &self,
        scope: &'l Mod,
        abs_path: &Path,
        label: &str,
        symbols: &[String],
    ) -> Result<(), EvalError> {
        let frozen = self.eval_file(scope, abs_path)?;
        for symbol in symbols {
            if !frozen.has_name(symbol) {
                return Err(EvalError::UnknownError(anyhow!(
                    "task symbol {:?} not found in @{} module use_task({:?})",
                    symbol,
                    scope.name,
                    label
                )));
            }
            if !frozen.has_task(symbol) {
                return Err(EvalError::UnknownError(anyhow!(
                    "invalid use_task({:?}, {:?}) in @{} module",
                    label,
                    symbol,
                    scope.name
                )));
            }
        }
        self.register_tasks(&frozen, symbols.iter().map(String::as_str))
    }

    /// Thaw `symbols` out of `frozen` and insert them into the shared
    /// `TaskMap` — all of them or none.
    ///
    /// Every task is thawed before the first is inserted, so a file whose
    /// later symbol is missing leaves no half of itself behind for `aspect
    /// help` to list and `aspect <task>` to then fail on. Phase 1's whole
    /// bargain is that an unloadable file costs exactly what that file
    /// defines, which only holds if registration is all-or-nothing.
    fn register_tasks<'s>(
        &self,
        frozen: &FrozenModule,
        symbols: impl IntoIterator<Item = &'s str>,
    ) -> Result<(), EvalError> {
        let heap = self.heap();
        let task_map = self
            .tasks
            .downcast_ref::<TaskMap>()
            .expect("self.tasks is a TaskMap");

        let mut thawed = vec![];
        for symbol in symbols {
            let owned = frozen
                .get(symbol)
                .map_err(|_| EvalError::MissingSymbol(symbol.to_string()))?;
            let frozen_value = owned
                .value()
                .unpack_frozen()
                .expect("value from FrozenModule is always frozen");
            heap.access_owned_frozen_value(&owned);
            thawed.push(Task::from_frozen(frozen_value, heap));
        }
        for live_task in thawed {
            task_map.insert(heap.alloc(live_task));
        }
        Ok(())
    }

    /// Register the feature one `use_feature(label, symbol)` entry names.
    fn eval_used_feature(
        &self,
        scope: &'l Mod,
        abs_path: &Path,
        symbol: &str,
    ) -> Result<(), EvalError> {
        let heap = self.heap();
        let feature_map = self
            .features
            .downcast_ref::<FeatureMap>()
            .expect("self.features is a FeatureMap");

        let frozen = self.eval_file(scope, abs_path)?;
        let owned = frozen
            .get(symbol)
            .map_err(|_| EvalError::MissingSymbol(symbol.to_string()))?;
        let frozen_value = owned.value().unpack_frozen().ok_or_else(|| {
            EvalError::UnknownError(anyhow!(
                "symbol {:?} in {:?} did not freeze",
                symbol,
                abs_path
            ))
        })?;
        if frozen_value.downcast_ref::<FrozenFeature>().is_none() {
            return Err(EvalError::UnknownError(anyhow!(
                "symbol {:?} in {:?} is not a feature",
                symbol,
                abs_path
            )));
        }
        // Register the frozen heap as a dependency of the live heap so the
        // FrozenValue back-pointer (kept on the thawed Feature) stays valid.
        heap.access_owned_frozen_value(&owned);
        let live_feature = Feature::from_frozen(frozen_value, heap);
        feature_map.insert(heap.alloc(live_feature));
        Ok(())
    }

    /// Snapshot of all tasks discovered in Phase 1 (plus any added during Phase 2 via
    /// `ctx.tasks.add(...)`), borrowed as `&dyn TaskLike` so callers don't have to
    /// downcast `Value`s themselves.
    pub fn tasks(&self) -> Vec<&'v dyn TaskLike<'v>> {
        self.tasks
            .downcast_ref::<TaskMap>()
            .expect("self.tasks is a TaskMap")
            .values()
            .into_iter()
            .map(|v| -> &'v dyn TaskLike<'v> {
                if let Some(t) = v.downcast_ref::<Task<'v>>() {
                    return t;
                }
                if let Some(t) = v.downcast_ref::<FrozenTask>() {
                    return t;
                }
                panic!("TaskMap entry is not a task: {}", v.get_type())
            })
            .collect()
    }

    /// The command path of every task in [`Self::tasks`], sorted: what a
    /// `task_presets` or `task_templates` key is checked against.
    pub fn task_paths(&self) -> Vec<String> {
        let mut paths: Vec<String> = self
            .tasks()
            .into_iter()
            .map(|t| task_command_path(t.group(), &t.kind()))
            .collect();
        paths.sort();
        paths.dedup();
        paths
    }

    /// Snapshot of all features discovered in Phase 1, borrowed as `&dyn FeatureLike`.
    pub fn features(&self) -> Vec<&'v dyn FeatureLike<'v>> {
        self.features
            .downcast_ref::<FeatureMap>()
            .expect("self.features is a FeatureMap")
            .values()
            .into_iter()
            .map(|v| -> &'v dyn FeatureLike<'v> {
                if let Some(f) = v.downcast_ref::<Feature<'v>>() {
                    return f;
                }
                if let Some(f) = v.downcast_ref::<FrozenFeature>() {
                    return f;
                }
                panic!("FeatureMap entry is not a feature: {}", v.get_type())
            })
            .collect()
    }

    /// Phase 2: evaluate config files against the task and feature values from Phase 1.
    ///
    /// Configs write trait instances into the run's shared `TraitMap`
    /// (`ctx.traits`), which features and the task read. Config functions mutate `Task`
    /// and `Feature` values in-place via `set_attr` — the same `Value<'v>` objects
    /// discovered in Phase 1 are reused, so subsequent phases see the updated state.
    /// `ctx.tasks.add(...)` may insert additional tasks into `self.tasks`.
    ///
    /// Each config is an absolute path paired with the module scope its `load()`
    /// statements resolve against. Files are evaluated in order, so a later
    /// file's writes win over an earlier one's — the user-global
    /// `~/.aspect/config.axl` is passed last, scoped to its own module so its
    /// relative loads resolve within `~/.aspect` rather than the project.
    ///
    /// As in Phase 1, a config that fails is recorded in
    /// [`PhaseFailures::files`] and the remaining ones still run, so a typo in
    /// one `config.axl` leaves the CLI usable enough to find it; one that
    /// [`Self::ends_the_run`] stops the phase as [`PhaseFailures::ended`]. So
    /// does the trait type check at the end, which is the one failure no
    /// single file owns — it judges the state all configs together were
    /// supposed to produce. It is skipped once any config failed, because it
    /// would then report the gap rather than the typo that caused it.
    ///
    /// Because the tolerance is per file, this phase runs even when Phase 1
    /// could not load every script. Three consequences. A config is evaluated
    /// against a task map that may be missing entries, so its own failure is
    /// often just that — `ctx.tasks["x"]` for an `x` whose file never loaded —
    /// and a caller should treat a config failure accompanying a script
    /// failure as derived from it rather than as a second, independent
    /// problem. Mutations a config made before failing stay on the shared
    /// heap, this phase's whole purpose being to mutate it. And a config's
    /// side effects — a file it writes, a request it sends — can happen in a
    /// workspace whose scripts did not all load.
    ///
    /// The policy a CLI builds on top of all this — which commands may run
    /// against a workspace that did not fully load, and what each failure is
    /// reported as — lives in `docs/design.md`, "When a file does not load".
    #[tracing::instrument(name = "execute.configs", skip_all)]
    pub fn execute_configs(&mut self, configs: &[(&Path, &'l Mod)]) -> PhaseFailures {
        let heap = self.heap();

        let context_value = heap.alloc(ConfigContext::new(
            self.tasks,
            self.trait_map_value,
            self.features,
            self.telemetry_value,
            self.hooks_value,
            self.loader.env.signals.clone(),
            self.loader.env.stream_color,
        ));

        let mut out = PhaseFailures::default();
        for (config_path, scope) in configs {
            if let Err(error) = self.run_config(scope, config_path, context_value) {
                // Stop only once an ending error has been seen. An ordinary
                // failure is recorded and the remaining configs still run.
                if self.record(&mut out, config_path, AxlFileRole::Config, error) {
                    break;
                }
            }
        }
        // What the configs set through `ctx.theme`, even those that then
        // failed: a theme is never worth failing over.
        let env = &self.loader.env;
        if let Ok(layers) = env.theme_config.lock() {
            crate::theme::refresh(&env.theme_inputs, &layers, &env.theme);
        }
        if !out.is_empty() {
            return out;
        }

        // Assignment to a trait field is type-checked as it happens, but a list
        // field mutated in place (`score_slot.append(5)`) is not, so every
        // instance a config touched is checked once they have all run.
        let trait_map = self
            .trait_map_value
            .downcast_ref::<TraitMap>()
            .expect("self.trait_map_value is a TraitMap");
        let mismatches: Vec<String> = trait_map
            .entries()
            .into_iter()
            .filter_map(|(_, _, instance)| instance.downcast_ref::<TraitInstance>())
            .flat_map(|instance| instance.type_mismatches())
            .collect();
        if !mismatches.is_empty() {
            out.ended = Some(EvalError::TraitTypeMismatch(mismatches));
        }

        out
    }

    /// Call one config file's `config(ctx)`. One unit of Phase 2's per-file
    /// fault tolerance — see [`Self::execute_configs`].
    fn run_config(
        &self,
        scope: &'l Mod,
        config_path: &Path,
        context_value: Value<'v>,
    ) -> Result<(), EvalError> {
        let heap = self.heap();
        let frozen = self.eval_file(scope, config_path)?;

        let def = frozen
            .get("config")
            .map_err(|_| EvalError::MissingConfig(config_path.to_path_buf()))?;
        let func = heap.access_owned_frozen_value(&def);

        let mut eval = Evaluator::new(&self.env.0);
        eval.set_print_handler(&crate::out::TOLERANT_PRINT_HANDLER);
        eval.set_loader(self.loader);
        eval.extra = Some(&self.loader.env);
        arm_cancel_check(&mut eval, &self.loader.env.signals);
        eval.eval_function(func, &[context_value], &[])?;
        Ok(())
    }

    /// Resolve the terminal surface for the task at `task_index` from the
    /// builtin `Console` feature's merged args (`args_builder` applies the CLI
    /// and `config.axl` precedence, as for `execute_features_with_args`).
    ///
    /// Runs after `execute_configs` and before the header, so a config's
    /// templates frame the whole run. Without the feature the run gets the
    /// `full` layout, and links to files open as
    /// `ASPECT_FILE_HYPERLINK_FORMAT` or the `config.json` files' `editor` says
    /// (`crate::out::set_unconfigured_link_format`); see
    /// [`console_value::from_feature_args`] for the rest, including the
    /// `agent` layout a coding agent gets. Color, Buildkite and coding-agent
    /// detection and the environment's verbose level come from the run's
    /// `Env` ([`Host::of`], which fails on an invalid `ASPECT_VERBOSE`), the
    /// console's only view of its host; the `editor` of the `config.json`
    /// files comes from the `Env` too (`config_editor`). Also
    /// sets the verbose level for the rest of the run ([`diag::set_verbose`])
    /// as the resolved console says.
    pub fn resolve_console(
        &self,
        task_index: usize,
        task_name: &str,
        args_builder: impl Fn(&dyn FeatureLike<'v>, Heap<'v>) -> Arguments<'v>,
    ) -> Result<Arc<Console>, EvalError> {
        let task = *self.tasks().get(task_index).ok_or_else(|| {
            EvalError::UnknownError(anyhow!("task index {} out of range", task_index))
        })?;
        let identity = TaskIdentity {
            kind: task.kind(),
            group: task.group().clone(),
            name: task_name.to_owned(),
            path: task_command_path(task.group(), &task.kind()),
        };
        let host = Host::of(&self.loader.env).map_err(|e| EvalError::UnknownError(anyhow!(e)))?;
        let console = match self
            .features()
            .into_iter()
            .find(|f| f.name() == CONSOLE_FEATURE)
        {
            None => {
                crate::out::set_unconfigured_link_format((self.loader.env.config_editor)());
                Console::builtin(identity, host)
            }
            Some(feature) => console_value::from_feature_args(
                &args_builder(feature, self.heap()),
                identity,
                host,
                &self.task_paths(),
                (self.loader.env.config_editor)(),
            )
            .map_err(|e| EvalError::UnknownError(anyhow!(e)))?,
        };
        let console = console.with_palette(&self.loader.env.theme.get());
        diag::set_verbose(console.verbosity());
        Ok(Arc::new(console))
    }

    /// Print the banner and the task's opening `header` slot through
    /// `console`, for the task at `task_index` in `self.tasks()`, then the
    /// console's own config warnings ([`Console::warnings`]). What is written
    /// from here on is this task's output, no earlier task's
    /// ([`crate::out::begin_early`]).
    ///
    /// Called from the CLI before `execute_features_with_args` so that any
    /// diagnostic output emitted during feature initialization (auth WARNINGs,
    /// tip blocks, etc.) is framed by the task header. Without this, the header
    /// would only appear after features finish — and feature-init output would
    /// look like it preceded any task running.
    pub fn print_running_task_header(
        &self,
        console: &Console,
        task_index: usize,
        task_name: &str,
        task_name_meaningful: bool,
    ) -> Result<(), EvalError> {
        let task = *self.tasks().get(task_index).ok_or_else(|| {
            EvalError::UnknownError(anyhow!("task index {} out of range", task_index))
        })?;
        crate::out::begin_early();
        let color = console.color();
        let (verb_seq, reset) = role_style(&self.loader.env.theme.get(), "highlight", color);
        let label = task_label(task.group(), &task.kind(), task_name, task_name_meaningful);
        // Identity banner above the task header — see `crate::banner`.
        if banner::show_runtime_banner() {
            let palette = self.loader.env.theme.get();
            let palette = color.then_some(&*palette);
            console.emit(Slot::Banner, banner::console_context(palette));
        }
        console.emit(
            Slot::Header,
            minijinja::context! { label => label, highlight_style => verb_seq, highlight_reset => reset },
        );
        for warning in console.warnings() {
            crate::diag::warn(warning);
        }
        Ok(())
    }

    /// Phase 3: run enabled feature implementations.
    ///
    /// Must be called after [`Self::execute_configs`] so that config files have had
    /// the opportunity to enable or disable features. `args_builder` builds the
    /// fully-merged `Arguments` for each feature (callers handle CLI/override/default
    /// precedence). Features whose resolved `enabled` is `False` are skipped.
    ///
    /// A failing feature impl propagates as its own `starlark::Error` under a
    /// `Feature implementation failed` context, so a [`TaskExit`] raised inside it
    /// is still found by [`TaskExit::from_anyhow`].
    #[tracing::instrument(name = "execute.features", skip_all)]
    pub fn execute_features_with_args(
        &mut self,
        args_builder: impl Fn(&dyn FeatureLike<'v>, Heap<'v>) -> Arguments<'v>,
    ) -> Result<(), EvalError> {
        let heap = self.heap();

        let trait_map_value = self.trait_map_value;
        let task_paths = Arc::new(self.task_paths());

        for feature in self.features() {
            let args = args_builder(feature, heap);

            let enabled = args
                .get("enabled")
                .and_then(|v| v.unpack_bool())
                .unwrap_or(true);
            if !enabled {
                continue;
            }

            let attrs_value = heap.alloc(args);
            let fctx = heap.alloc(FeatureContext::new(
                attrs_value,
                trait_map_value,
                self.telemetry_value,
                self.hooks_value,
                self.loader.env.signals.clone(),
                self.loader.env.stream_color,
                task_paths.clone(),
            ));
            let mut eval = Evaluator::new(&self.env.0);
            eval.set_print_handler(&crate::out::TOLERANT_PRINT_HANDLER);
            eval.set_loader(self.loader);
            eval.extra = Some(&self.loader.env);
            arm_cancel_check(&mut eval, &self.loader.env.signals);
            eval.eval_function(feature.implementation(), &[fctx], &[])
                .map_err(|e| {
                    EvalError::UnknownError(
                        anyhow::Error::from(EvalError::StarlarkError(e))
                            .context("Feature implementation failed"),
                    )
                })?;
        }

        Ok(())
    }

    /// Phase 4: execute the task at the given index with pre-built args.
    ///
    /// The index refers to the position in `self.tasks()` (same Vec returned to
    /// the CLI when building the command tree). `args_builder` returns the
    /// fully-merged `Arguments` (callers handle CLI/override/default precedence).
    /// `console` is the surface from [`Self::resolve_console`]; it backs
    /// `ctx.console` and renders the bookend.
    ///
    /// Runs, in order: the pre-task hooks, `_impl`, the post-task hooks with
    /// the resolved conclusion, the `ctx.defer` callbacks, and the closing
    /// bookend, the console's `verdict` slot (see [`TaskHooks`]). Returns the
    /// task's exit code: what `_impl` returned, or the code of a [`TaskExit`]
    /// it or a pre-task hook raised,
    /// which is reported without a traceback. Any other error propagates once
    /// the hooks and defers have run, with no bookend.
    #[tracing::instrument(
        name = "execute.task",
        skip_all,
        err,
        fields(
            task_name = %task_name,
            task = tracing::field::Empty,
            task_id = tracing::field::Empty,
            exit_code = tracing::field::Empty,
        )
    )]
    pub fn execute_tasks_with_args(
        &mut self,
        task_index: usize,
        task_name: String,
        task_name_meaningful: bool,
        task_friendly_name: Option<String>,
        task_id: Option<String>,
        timing: TimingMode,
        console: Arc<Console>,
        args_builder: impl FnOnce(&dyn TaskLike<'v>, Heap<'v>) -> Arguments<'v>,
    ) -> Result<Option<u8>, EvalError> {
        let task = *self.tasks().get(task_index).ok_or_else(|| {
            EvalError::UnknownError(anyhow!("task index {} out of range", task_index))
        })?;

        let heap = self.heap();
        // A task ID names one run: one already in this machine's history is
        // refused. A generated one is fresh by construction.
        if let Some(id) = &task_id
            && let Some(earlier) = self.loader.env.history.earlier_run(id)
        {
            return Err(EvalError::UnknownError(anyhow!(
                "--task:id {id} was already used by an earlier run on this machine ({}); a \
                 task ID names one run, so pass a new one",
                earlier.display(),
            )));
        }
        let task_id = task_id.unwrap_or_else(|| Uuid::new_v4().to_string());
        let env = &self.loader.env;
        env.history.begin_task(
            &task_id,
            HistoryTaskStart {
                name: task_name.clone(),
                kind: task.kind(),
                group: task.group().clone(),
                cli_version: env.cli_version.clone(),
                workspace: Some(env.bazel_root_dir.clone()),
                git_root: env.git_root_dir.clone(),
            },
        );
        let task_args = args_builder(task, heap);

        let span = tracing::Span::current();
        span.record("task", task.kind());
        span.record("task_id", task_id.as_str());

        // Resolve the per-invocation friendly name: explicit --task:friendly-name,
        // else the task name verbatim.
        let task_friendly_name = task_friendly_name.unwrap_or_else(|| task_name.clone());

        // Capture the kind + group early — `task_info` is moved into TaskContext
        // below and is no longer accessible from the closing announcement after
        // the eval returns.
        let task_kind = task.kind();
        let task_group = task.group().clone();
        let task_info = TaskInfo::new(
            task.kind(),
            task.friendly_kind(),
            task.group().clone(),
            task_name.clone(),
            task_friendly_name,
            task_id,
        );

        // The header printed before the features ran; only the verdict is
        // rendered here, in colors matching the header's.
        let color = console.color();
        let palette = self.loader.env.theme.get();
        let (success_style, _) = role_style(&palette, "label.success", color);
        let (warning_style, _) = role_style(&palette, "label.warning", color);
        let (error_style, _) = role_style(&palette, "label.error", color);
        let reset = if color { aspect_theme::RESET } else { "" };

        // The task shares the run's map. Every exported trait type is
        // registered (not instantiated) so the console finds `BazelTrait`
        // by its fields even when nothing has read it yet.
        let trait_map = self
            .trait_map_value
            .downcast_ref::<TraitMap>()
            .expect("self.trait_map_value is a TraitMap");
        for owned in self.loader.exported_trait_types() {
            let type_value = heap.access_owned_frozen_value(&owned).to_value();
            if let Some(id) = extract_trait_type_id(type_value) {
                trait_map.insert(id, type_value);
            }
        }
        let task_trait_map = self.trait_map_value;

        // Allocate `task_info` on the heap and keep the resulting Value
        // so we can read back timing + phases after `_impl` returns.
        // `TaskInfo::started_at` is the authoritative task start (stamped
        // in `TaskInfo::new`), so we don't need a stack-local `task_start`.
        let task_info_val = heap.alloc(task_info);
        // Keep the args Value: the claim record is read back off it before every
        // Bazel invocation and once more after `_impl` returns.
        let task_args_val = heap.alloc(task_args);
        let bazel = heap.alloc(Bazel {
            active_rc: std::cell::RefCell::new(None),
            claims: ClaimCheck {
                task_kind: task_kind.clone(),
                buckets: passthrough::bucket_names(task),
                args: task_args_val,
            },
        });
        let context = heap.alloc(TaskContext::new(
            task_args_val,
            task_trait_map,
            task_info_val,
            bazel,
            self.hooks_value,
            self.loader.env.signals.clone(),
            self.loader.env.stream_color,
            console.clone(),
        ));

        let mut eval = Evaluator::new(&self.env.0);
        eval.set_print_handler(&crate::out::TOLERANT_PRINT_HANDLER);
        eval.set_loader(self.loader);
        eval.extra = Some(&self.loader.env);
        arm_cancel_check(&mut eval, &self.loader.env.signals);
        crate::engine::module_values::set_task_context(&eval, context);

        let hooks = self
            .hooks_value
            .downcast_ref::<TaskHooks>()
            .expect("hooks_value is a TaskHooks");
        hooks.start();
        let body_result = match hooks.run_pre(context, &mut eval) {
            Ok(()) => eval.eval_function(task.implementation(), &[context], &[]),
            Err(e) => Err(e),
        };
        let (mut outcome, ending) = Outcome::resolve(body_result, &self.loader.env.signals);
        // The body has ended: from here the post-task hooks, defers and
        // bookend run to completion whatever the root token says. Starlark
        // runs the cancel check after every `eval_function`, so an armed
        // check would fail each hook as it returned; and blocking builtins
        // must block normally so cleanup can wait on things.
        eval.set_check_cancelled(Box::new(|| false));
        self.loader.env.signals.enter_unwind();
        // A hard error prints itself, traceback and all, once it propagates
        // below; an exit or return is reported here like a conclusion.
        if !matches!(ending, Ending::Failed(_)) {
            outcome.report_message();
            if let Ending::Exited(e) = &ending {
                TaskExit::debug_traceback(e);
            }
            outcome.exit_code =
                apply_unclaimed_flags(task, &task_kind, task_args_val, outcome.exit_code);
        }
        let conclusion = heap.alloc(outcome.to_conclusion());
        hooks.run_post(context, conclusion, &mut eval);
        run_deferred(context, &mut eval);
        let cancelled = self.loader.env.signals.root().is_cancelled();
        if let Ending::Failed(e) = ending {
            let error = e.to_string();
            self.loader.env.history.end_task(
                None,
                String::new(),
                crate::engine::task_record::error_line(&error),
                cancelled,
            );
            return Err(e.into());
        }
        self.loader.env.history.end_task(
            Some(outcome.exit_code.unwrap_or(0)),
            outcome.text.clone(),
            String::new(),
            cancelled,
        );
        let Outcome {
            exit_code,
            flagged,
            text: conclusion,
            ..
        } = outcome;

        // Reads elapsed + phases off the heap-allocated TaskInfo. Closes
        // any phase still active when `_impl` returned so the breakdown
        // is whole.
        let (elapsed, phases) = {
            let info = task_info_val
                .downcast_ref::<TaskInfo>()
                .expect("task_info_val downcasts to TaskInfo");
            info.close_active_phase();
            (info.started_at.elapsed(), info.phases.borrow().clone())
        };

        tracing::Span::current().record("exit_code", exit_code.unwrap_or(0) as i64);
        if let Some(code) = exit_code
            && code != 0
        {
            tracing::error!(exit_code = code as i64, "task exited with non-zero status");
        }

        let failed = matches!(exit_code, Some(code) if code != 0);
        let exit_suffix = match exit_code {
            Some(code) if code != 0 => format!(" (exit {})", code),
            _ => String::new(),
        };
        let label = task_label(&task_group, &task_kind, &task_name, task_name_meaningful);
        let verdict = Verdict::pick(
            exit_code,
            flagged,
            &success_style,
            &warning_style,
            &error_style,
        );
        let plain_breakdown = render_phase_breakdown(&phases, timing, failed, false);
        let context = minijinja::context! {
            verdict => verdict.name,
            cancelled => exit_code == Some(INTERRUPTED_EXIT_CODE),
            glyph => verdict.glyph,
            shortcode => verdict.bk_shortcode,
            verb => verdict.verb,
            highlight_style => verdict.color,
            highlight_reset => reset,
            label => label,
            exit_code => exit_code.unwrap_or(0),
            exit => exit_suffix,
            elapsed => format_duration(elapsed),
            elapsed_ms => elapsed.as_millis() as u64,
            timing => render_timing_segment(timing, elapsed),
            conclusion => conclusion,
            breakdown => render_phase_breakdown(&phases, timing, failed, true),
            timings => dotted(&plain_breakdown),
            plain_breakdown => plain_breakdown,
            phases => phases.iter().map(phase_context).collect::<Vec<_>>(),
        };
        if console.buildkite() && (failed || flagged) {
            // On a non-clean verdict, retroactively expand the section that is
            // still open — the last phase the task emitted, i.e. the one that
            // failed or got flagged. BK's `^^^ +++` expands the currently-open
            // section regardless of how its `---` header declared it; emitting
            // it here (before the closing bookend opens its own section) targets
            // that phase. We can't know a phase will fail/flag when its `---`
            // header is printed, so this is the only point where the verdict is
            // known and the offending section is still current.
            //
            // `^^^ +++` counts as an explicit expansion, which defeats BK's
            // "auto-expand the last collapsed `---` group when nothing is
            // explicitly expanded" rule — so the timing-summary bookend below
            // would collapse. Open it with `+++` on this path so both the
            // failing phase and the summary stay expanded. On a clean run
            // `emit` keeps `---` and BK's auto-expand handles the summary.
            errln!("^^^ +++");
            console.emit_section("+++", Slot::Verdict, context);
        } else if console.buildkite() {
            // Expanded on a clean run too: a phase that runs Bazel opens its
            // section `+++`, which keeps BK from auto-expanding the last
            // collapsed one, so a `---` summary would stay shut.
            console.emit_section("+++", Slot::Verdict, context);
        } else {
            console.emit(Slot::Verdict, context);
        }

        Ok(exit_code)
    }

    /// The command line the task runs with, as typed, the spellings of its
    /// flags whose values are secrets, and its other short flags that take a
    /// value, for its history record.
    pub fn set_task_command_line(
        &self,
        argv: Vec<String>,
        secret_flags: Vec<String>,
        value_shorts: Vec<String>,
    ) {
        self.loader
            .env
            .history
            .set_command_line(argv, secret_flags, value_shorts);
    }

    pub fn finish(self) -> FinishedEval {
        FinishedEval
    }
}

/// The exit code once the passthrough-flag check has had its say. A task that
/// declared a bucket promised to act on what the CLI routed into it, so
/// unclaimed flags fail an otherwise-successful run; a task that failed on its
/// own has the more informative story and keeps its code, with the flags only
/// warned about. An early exit is treated like the return it stands in for.
fn apply_unclaimed_flags<'v>(
    task: &dyn TaskLike<'v>,
    task_kind: &str,
    task_args_val: Value<'v>,
    exit_code: Option<u8>,
) -> Option<u8> {
    let unclaimed = passthrough::unclaimed(&passthrough::bucket_names(task), task_args_val);
    if unclaimed.is_empty() {
        return exit_code;
    }
    let failed_on_its_own = matches!(exit_code, Some(code) if code != 0);
    for (bucket, flags) in &unclaimed {
        let message = passthrough::message(task_kind, "return", bucket, flags);
        if failed_on_its_own {
            diag::warn(&message);
        } else {
            diag::error(&message);
        }
    }
    if failed_on_its_own {
        exit_code
    } else {
        Some(passthrough::EXIT_UNCLAIMED)
    }
}

/// The exit code of a task cut short by an interrupt (128 + SIGINT), which
/// the verdict shows as cancelled.
const INTERRUPTED_EXIT_CODE: u8 = 130;

/// Make the evaluator end at the next loop iteration once the root token is
/// cancelled, as `ctx.std.process.exit(130, "interrupted")` would: the safe
/// point for pure Starlark loops. Blocking builtins have their own.
fn arm_cancel_check<'a>(eval: &mut Evaluator<'_, 'a, '_>, signals: &'a Signals) {
    eval.set_check_cancelled(Box::new(|| signals.should_unwind()));
}

fn run_deferred<'v>(context: Value<'v>, eval: &mut Evaluator<'v, '_, '_>) {
    let Some(ctx) = context.downcast_ref::<TaskContext>() else {
        return;
    };
    let defers = ctx.drain_defers();
    for defer in defers {
        if let Err(e) = eval.eval_function(defer.callable, &defer.args, &defer.kwargs) {
            task_hooks::report_callback_failure("ctx.defer callback", &e);
        }
    }
}

/// Three-way verdict rendered on the closing bookend line. Selected
/// from the task's exit code and `flagged` bit:
///
///   exit 0, !flagged → Passed   (✓ in `full`, ✅ in `emoji`)
///   exit 0,  flagged → Flagged  (✓ in `full`, ⚠️ in `emoji`)
///   exit non-0       → Failed   (✗ in `full`, ❌ in `emoji`)
///
/// On BK the closing line lands as its own section header, always `+++`
/// (expanded), so the timing summary stays open alongside an expanded
/// failing phase or one that ran Bazel.
struct Verdict<'a> {
    /// `passed` | `flagged` | `failed`, the `verdict` key of the template context.
    name: &'a str,
    /// BK emoji shortcode (e.g. `:white_check_mark:`); used in the
    /// `--- :<shortcode>: <verb> …` BK section header.
    bk_shortcode: &'a str,
    /// Unicode glyph (e.g. `✅`); used off Buildkite. `⚠️` carries a
    /// trailing space — it pre-dates the emoji standard and renders
    /// 1-cell-wide in some terminals where ✅/❌ are reliably 2-cell.
    /// The space keeps the line aligned across verdicts.
    glyph: &'a str,
    verb: &'a str,
    /// The escape opening the verdict's `label.*` role. Empty when the console's
    /// [`crate::console::Console::color`] is off. Paired with `reset` at the
    /// call site.
    color: &'a str,
}

impl<'a> Verdict<'a> {
    fn pick(
        exit_code: Option<u8>,
        flagged: bool,
        success_style: &'a str,
        warning_style: &'a str,
        error_style: &'a str,
    ) -> Self {
        match (exit_code, flagged) {
            (Some(0) | None, false) => Self {
                name: "passed",
                bk_shortcode: ":white_check_mark:",
                glyph: "✅",
                verb: "Passed",
                color: success_style,
            },
            (Some(0) | None, true) => Self {
                name: "flagged",
                bk_shortcode: ":warning:",
                glyph: "⚠️ ",
                verb: "Flagged",
                color: warning_style,
            },
            (Some(_), _) => Self {
                name: "failed",
                bk_shortcode: ":x:",
                glyph: "❌",
                verb: "Failed",
                color: error_style,
            },
        }
    }
}

/// A phase's display label: its `display_name`, else its title-cased name.
fn phase_title(p: &PhaseRecord) -> String {
    if p.display_name.is_empty() {
        titlecase(&p.name)
    } else {
        p.display_name.clone()
    }
}

/// One entry of the `verdict` slot's `phases` list.
fn phase_context(p: &PhaseRecord) -> minijinja::Value {
    minijinja::context! {
        name => p.name,
        title => phase_title(p),
        emoji => p.emoji,
        description => p.description,
        duration => format_duration(p.duration),
        duration_ms => p.duration.as_millis() as u64,
        interrupted => p.interrupted,
    }
}

/// Format an elapsed `Duration` as a short human-readable string.
///
/// Three regimes for legibility:
///   < 1s     → `"500ms"`, `"5ms"`     — millisecond precision so tiny
///                                       phases don't all read as "0.0s"
///   < 1m     → `"4.2s"`, `"59.9s"`    — tenth-of-a-second precision
///   ≥ 1m     → `"1m 5s"`, `"61m 1s"`  — whole seconds inside minutes
///
/// Mirrors AXL's `bazel_results.format_duration_ms` so the closing
/// CLI marker reads the same as durations rendered inside per-task
/// summaries.
fn format_duration(d: std::time::Duration) -> String {
    let total_ms = d.as_millis();
    if total_ms < 1_000 {
        format!("{}ms", total_ms)
    } else if total_ms < 60_000 {
        let secs = total_ms / 1000;
        let tenths = (total_ms % 1000) / 100;
        format!("{}.{}s", secs, tenths)
    } else {
        let total_s = (total_ms / 1000) as u64;
        format!("{}m {}s", total_s / 60, total_s % 60)
    }
}

/// Threshold below which the synthetic `init` phase is hidden from
/// the breakdown. The init phase covers Aspect runtime startup +
/// trait setup before the task's first explicit `phase()` mark; on
/// a fast path that's a few ms of work and not worth a row in the
/// breakdown. Above this threshold, init surfaces as a real entry
/// the user might want to investigate (cold-start anomalies, slow
/// trait construction, etc.). Caller-named phases always render
/// even at 0ms — those are intentional boundaries.
const INIT_PHASE_HIDE_THRESHOLD_MS: u128 = 50;

/// Verbosity for the timing summary trailing the runtime's
/// "✓ Passed" / "✓ Flagged" / "✗ Failed" line. Driven by the
/// top-level `--task:timing-summary` CLI flag.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TimingMode {
    /// No timing summary at all — not even the total.
    /// `✓ Passed lint task`
    None,
    /// Total only.
    /// `✓ Passed lint task in 46.8s`
    Total,
    /// Total + inline phase breakdown.
    /// `✓ Passed lint task in 46.8s · Init 0.2s · Detect 1.2s · ...`
    Short,
    /// Total + multi-line breakdown with descriptions. Default.
    Detailed,
}

impl Default for TimingMode {
    fn default() -> Self {
        TimingMode::Detailed
    }
}

impl std::str::FromStr for TimingMode {
    type Err = String;
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "none" => Ok(TimingMode::None),
            "total" => Ok(TimingMode::Total),
            "short" => Ok(TimingMode::Short),
            "detailed" => Ok(TimingMode::Detailed),
            other => Err(format!(
                "invalid timing mode {:?}{}",
                other,
                crate::suggest::did_you_mean_then(
                    other,
                    ["none", "total", "short", "detailed"],
                    "; ",
                    "expected one of: none, total, short, detailed"
                )
            )),
        }
    }
}

/// Render the `" in <duration>"` segment of the task completion line.
///
/// Empty for `TimingMode::None` (no timing summary at all); every other
/// level keeps the total. The leading space lets it concatenate after the
/// `task` token.
fn render_timing_segment(mode: TimingMode, elapsed: std::time::Duration) -> String {
    if mode == TimingMode::None {
        String::new()
    } else {
        format!(" in {}", format_duration(elapsed))
    }
}

/// `breakdown`, a [`render_phase_breakdown`] in `Short` mode, led by ` · `
/// rather than ` — `; any other form as it is.
fn dotted(breakdown: &str) -> String {
    match breakdown.strip_prefix(" — ") {
        Some(rest) => format!(" · {rest}"),
        None => breakdown.to_owned(),
    }
}

/// Render the phase breakdown that trails the runtime's closing verdict
/// line.
///
/// Returns `""` for `None`/`Total` mode, or for any mode when `phases`
/// is empty (task didn't opt into phases).
///
/// `Short` returns a leading-space-and-em-dash inline form so it
/// concatenates cleanly after the duration. `Detailed` returns a
/// leading newline plus indented per-phase lines.
///
/// When `failed` is true, an `interrupted` phase entry (recorded by
/// `close_active_phase()` on `_impl` return) renders with a
/// `(failed after <duration>)` suffix instead of a bare duration —
/// it's the phase that was active when the task hit a non-zero exit.
/// On success or a non-failed exit, `interrupted` is treated as a
/// normal completed entry.
///
/// `emoji` prefixes each phase with its `Phase(emoji=...)`; without it the
/// rows carry the bare titles, still aligned.
fn render_phase_breakdown(
    phases: &[PhaseRecord],
    mode: TimingMode,
    failed: bool,
    emoji: bool,
) -> String {
    if phases.is_empty() || mode == TimingMode::None || mode == TimingMode::Total {
        return String::new();
    }
    // Suppress the synthetic `init` phase when it's negligible. The
    // synthetic init covers the setup time before the task's first
    // explicit `phase()` call; a few ms of that is just statement
    // overhead, not signal. Caller-named phases (everything else)
    // always render, even at 0ms — those are intentional boundaries.
    let visible: Vec<&PhaseRecord> = phases
        .iter()
        .filter(|p| !(p.name == "init" && p.duration.as_millis() < INIT_PHASE_HIDE_THRESHOLD_MS))
        .collect();
    if visible.is_empty() {
        return String::new();
    }
    // The Detailed branch aligns columns by `display_width` (which counts the
    // emoji as two cells); Short mode has no columns.
    let phase_label = |p: &PhaseRecord| -> String {
        let name = phase_title(p);
        if !emoji || p.emoji.is_empty() {
            name
        } else {
            format!("{} {}", p.emoji, name)
        }
    };
    let format_phase = |p: &PhaseRecord| -> String {
        let label = phase_label(p);
        let dur = format_duration(p.duration);
        if failed && p.interrupted {
            format!("{} (failed after {})", label, dur)
        } else {
            format!("{} {}", label, dur)
        }
    };
    match mode {
        TimingMode::None | TimingMode::Total => unreachable!(),
        TimingMode::Short => {
            let parts: Vec<String> = visible.iter().map(|p| format_phase(p)).collect();
            format!(" — {}", parts.join(" · "))
        }
        TimingMode::Detailed => {
            // Pad the label column to align the duration column. Widths are
            // in terminal display cells (see `display_width`) — the phase
            // emoji renders two cells wide but is one codepoint, so a naive
            // char or byte count would misalign rows with vs without emoji.
            let labels: Vec<String> = visible.iter().map(|p| phase_label(p)).collect();
            let name_w = labels.iter().map(|s| display_width(s)).max().unwrap_or(0);
            let dur_strs: Vec<String> = visible
                .iter()
                .map(|p| format_duration(p.duration))
                .collect();
            let dur_w = dur_strs.iter().map(|s| s.len()).max().unwrap_or(0);
            let mut out = String::new();
            for ((p, dur), label) in visible.iter().zip(dur_strs.iter()).zip(labels.iter()) {
                let desc_base = if p.description.is_empty() {
                    "—"
                } else {
                    p.description.as_str()
                };
                let failure_marker = if failed && p.interrupted {
                    " (failed)"
                } else {
                    ""
                };
                let pad = name_w.saturating_sub(display_width(label));
                out.push_str(&format!(
                    "\n    {}{}  {:>dw$}  {}{}",
                    label,
                    " ".repeat(pad),
                    dur,
                    desc_base,
                    failure_marker,
                    dw = dur_w,
                ));
            }
            out
        }
    }
}

/// Title-case the first ASCII letter, convert underscores to spaces.
///
/// Phase names are lowercase identifiers (`detect`, `build`,
/// `bazel_query`, `init`); this renders them as `Detect`, `Build`,
/// `Bazel query`, `Init` for display. Callers that need a non-naive
/// display label pass an explicit `display_name` to
/// `ctx.task.phase(...)`; renderers prefer that over this helper.
fn titlecase(s: &str) -> String {
    let mut chars = s.chars();
    match chars.next() {
        Some(c) => {
            let mut out = String::with_capacity(s.len());
            out.extend(c.to_uppercase());
            for ch in chars {
                if ch == '_' {
                    out.push(' ');
                } else {
                    out.push(ch);
                }
            }
            out
        }
        None => String::new(),
    }
}

/// Terminal display width of `s`, in cells, for column alignment.
///
/// Phase labels are `<emoji> <ascii-name>` or a bare ASCII name. ASCII is
/// one cell; the variation selector U+FE0F is zero-width; every other
/// (non-ASCII) char — i.e. the phase emoji — counts as two cells. This
/// avoids a full Unicode width table.
///
/// The two-cell assumption holds only because every phase emoji is one that
/// terminals render double-width: a single astral codepoint (`🔨` U+1F528
/// and friends) or a BMP symbol with default *emoji* presentation (`✨`
/// U+2728). It would mis-measure a symbol with default *text* presentation
/// — e.g. the gear U+2699 or warning U+26A0, which render one cell even with
/// a U+FE0F selector in many terminals — so phase emoji must be chosen from
/// the double-width set (see the `Phase(emoji=…)` call sites). The U+FE0F
/// case is handled as zero-width defensively; no current phase emoji uses it.
fn display_width(s: &str) -> usize {
    s.chars()
        .map(|c| match c {
            '\u{FE0F}' => 0,
            c if c.is_ascii() => 1,
            _ => 2,
        })
        .sum()
}

#[cfg(test)]
mod tests {
    use super::{
        ModuleEnv, MultiPhaseEval, TimingMode, display_width, format_duration,
        render_phase_breakdown, render_timing_segment, task_label_for,
    };
    use crate::engine::arguments::Arguments;
    use crate::engine::passthrough;
    use crate::engine::task_info::PhaseRecord;
    use crate::eval::Loader;
    use crate::module::Mod;
    use starlark::values::ValueLike;
    use std::time::Duration;

    #[test]
    fn a_near_miss_timing_mode_is_suggested() {
        let err = "shrot".parse::<TimingMode>().unwrap_err();
        assert!(
            err.ends_with(
                r#"; did you mean "short"? Expected one of: none, total, short, detailed"#
            ),
            "{err}"
        );
        let err = "verbose".parse::<TimingMode>().unwrap_err();
        assert!(!err.contains("did you mean"), "{err}");
    }

    /// A task declaring an `args.passthrough()` bucket promises to act on what
    /// the CLI routes into it. These four pin the whole contract: the task gets
    /// to run first (it may want to inspect the flags), a claim satisfies the
    /// promise, a bare read does not, and an empty bucket is never a complaint.
    /// Anything that reaches a Bazel server must be stopped *before* it does:
    /// the flags are missing from the command line we would run, and an
    /// unclaimed startup option means we would even be talking to the wrong
    /// server. Finding that out after a build costs the whole build.
    #[test]
    fn reaching_bazel_without_claiming_fails_before_the_spawn() {
        for call in [
            r#"ctx.bazel.build("//...", stdout = None, stderr = None).wait()"#,
            r#"ctx.bazel.query("//...")"#,
            r#"ctx.bazel.info()"#,
            "ctx.bazel.shutdown()",
            "ctx.bazel.health_check()",
            "ctx.bazel.recover_poisoned_sandbox()",
            "ctx.bazel.version()",
        ] {
            let code = format!(
                r#"
def _impl(ctx):
    {call}
    return 0

t = task(implementation = _impl, args = {{"rest": args.passthrough(position = "post_command")}})
"#
            );
            let err = crate::test::eval(&code)
                .with_fake_bazel()
                .with_string_list_args([("rest", vec!["--jobs=8"])])
                .run_task(0)
                .expect_err("the invocation must be refused");
            let message = format!("{err:?}");
            assert!(
                message.contains("--jobs=8") && message.contains("about to run bazel"),
                "{call}: expected the claim diagnostic, got: {message}",
            );
        }
    }

    /// Claiming first lets the same task through to the spawn.
    #[test]
    fn spawning_bazel_after_claiming_is_allowed() {
        let code = r#"
def _impl(ctx):
    _ = ctx.args.claim("rest")
    ctx.bazel.build("//...", stdout = None, stderr = None).wait()
    return 0

t = task(implementation = _impl, args = {"rest": args.passthrough(position = "post_command")})
"#;
        crate::test::eval(code)
            .with_fake_bazel()
            .with_string_list_args([("rest", vec!["--jobs=8"])])
            .run_task(0)
            .expect("a claimed bucket lets the invocation run");
    }

    const PASSTHROUGH_TASK: &str = r#"
def _claims(ctx):
    _ = ctx.args.claim("rest")
    return 0

def _reads(ctx):
    _ = ctx.args.rest
    return 0

def _fails(ctx):
    return 1

claims = task(implementation = _claims, args = {"rest": args.passthrough(position = "post_command")})
reads = task(implementation = _reads, args = {"rest": args.passthrough(position = "post_command")})
fails = task(implementation = _fails, args = {"rest": args.passthrough(position = "post_command")})
"#;

    fn run_passthrough_task(index: usize, routed: Vec<&str>) -> Option<u8> {
        crate::test::eval(PASSTHROUGH_TASK)
            .with_string_list_args([("rest", routed)])
            .run_task(index)
            .expect("task runs")
    }

    #[test]
    fn claiming_routed_flags_satisfies_the_contract() {
        assert_eq!(run_passthrough_task(0, vec!["--jobs=8"]), Some(0));
    }

    /// Reading is inspecting, not forwarding — the flags still went nowhere.
    #[test]
    fn reading_routed_flags_without_claiming_fails_the_task() {
        assert_eq!(
            run_passthrough_task(1, vec!["--jobs=8"]),
            Some(passthrough::EXIT_UNCLAIMED)
        );
    }

    /// The common case: a task declares a bucket, the user routed nothing into
    /// it, and nobody had to claim anything.
    #[test]
    fn an_empty_bucket_is_not_a_failure() {
        assert_eq!(run_passthrough_task(1, vec![]), Some(0));
    }

    /// A task that concluded with its own non-zero code keeps it: its failure is
    /// the more useful story, so the dropped flags are reported as a warning
    /// rather than replacing the exit code. (A task that hard-errors never
    /// reaches the check at all.)
    #[test]
    fn a_failing_task_keeps_its_own_exit_code() {
        assert_eq!(run_passthrough_task(2, vec!["--jobs=8"]), Some(1));
    }

    #[test]
    fn task_label_brackets_kind_when_name_carries_info() {
        let no_group: &[String] = &[];
        // Meaningful name (explicit --task:name or CI-derived) → `<name> [<kind>]`,
        // on or off CI.
        assert_eq!(
            task_label_for(no_group, "format", "format-repeat-2", true, false),
            "format-repeat-2 [format]"
        );
        // On CI, any distinct name is bracketed even if not flagged meaningful.
        assert_eq!(
            task_label_for(no_group, "test", "test-ci-linux", false, true),
            "test-ci-linux [test]"
        );
        // Local autogenerated placeholder (not meaningful, off CI) → just the kind.
        assert_eq!(
            task_label_for(no_group, "format", "format-brash-cherries", false, false),
            "format task"
        );
        // name == kind → no redundant bracket, even meaningful or on CI.
        assert_eq!(
            task_label_for(no_group, "lint", "lint", true, true),
            "lint task"
        );
    }

    #[test]
    fn task_label_prefixes_group_as_command_path() {
        let auth: &[String] = &["auth".to_string()];
        // A grouped task shows its full command path (space-joined), matching the
        // CLI invocation `aspect auth configure`.
        assert_eq!(
            task_label_for(auth, "configure", "configure-brash-cherries", false, false),
            "auth configure task"
        );
        // The bracketed-name form brackets the whole path.
        assert_eq!(
            task_label_for(auth, "configure", "my-run", true, false),
            "my-run [auth configure]"
        );
        // A top-level task (no group) is unchanged.
        assert_eq!(
            task_label_for(&[], "build", "build", false, false),
            "build task"
        );
    }

    fn phase(name: &str, emoji: &str, secs: u64) -> PhaseRecord {
        PhaseRecord {
            name: name.to_string(),
            description: format!("{name} desc"),
            duration: Duration::from_secs(secs),
            interrupted: false,
            emoji: emoji.to_string(),
            display_name: String::new(),
        }
    }

    #[test]
    fn timing_mode_from_str_parses_all_levels() {
        assert_eq!("none".parse::<TimingMode>(), Ok(TimingMode::None));
        assert_eq!("total".parse::<TimingMode>(), Ok(TimingMode::Total));
        assert_eq!("short".parse::<TimingMode>(), Ok(TimingMode::Short));
        assert_eq!("detailed".parse::<TimingMode>(), Ok(TimingMode::Detailed));
    }

    #[test]
    fn timing_mode_from_str_rejects_invalid() {
        let err = "verbose".parse::<TimingMode>().unwrap_err();
        assert!(
            err.contains("none, total, short, detailed"),
            "error should list valid levels, got: {err}"
        );
    }

    #[test]
    fn timing_segment_suppressed_for_none() {
        let d = Duration::from_secs(5);
        assert_eq!(render_timing_segment(TimingMode::None, d), "");
        for mode in [TimingMode::Total, TimingMode::Short, TimingMode::Detailed] {
            assert_eq!(
                render_timing_segment(mode, d),
                format!(" in {}", format_duration(d)),
                "{mode:?} should keep the total"
            );
        }
    }

    #[test]
    fn phase_breakdown_empty_for_none_and_total() {
        let phases = [phase("setup", "🔧", 2), phase("build", "🔨", 3)];
        assert_eq!(
            render_phase_breakdown(&phases, TimingMode::None, false, true),
            ""
        );
        assert_eq!(
            render_phase_breakdown(&phases, TimingMode::Total, false, true),
            ""
        );
    }

    #[test]
    fn phase_breakdown_without_emoji_keeps_bare_titles() {
        let phases = [phase("setup", "🔧", 2), phase("build", "🔨", 3)];
        let short = render_phase_breakdown(&phases, TimingMode::Short, false, false);
        assert!(short.starts_with(" — Setup "), "{short}");
        assert!(!short.contains('🔧') && !short.contains('🔨'), "{short}");
        let detailed = render_phase_breakdown(&phases, TimingMode::Detailed, false, false);
        let rows: Vec<&str> = detailed.lines().filter(|l| !l.is_empty()).collect();
        assert!(rows[0].trim_start().starts_with("Setup "), "{detailed}");
        assert!(rows[1].trim_start().starts_with("Build "), "{detailed}");
    }

    #[test]
    fn phase_breakdown_short_is_inline() {
        let phases = [phase("setup", "🔧", 2), phase("build", "🔨", 3)];
        let out = render_phase_breakdown(&phases, TimingMode::Short, false, true);
        assert!(out.starts_with(" — "), "Short form is inline; got: {out}");
        assert!(
            out.contains(" · "),
            "Short form joins phases with ·; got: {out}"
        );
    }

    #[test]
    fn display_width_counts_emoji_as_two_ascii_as_one() {
        assert_eq!(display_width("Setup"), 5);
        assert_eq!(display_width("🔨 Build"), 8); // astral emoji 2 + " " + "Build"
        assert_eq!(display_width("🔧 Setup"), 8); // wrench (astral) 2 + 1 + 5
        assert_eq!(display_width("✨ Format"), 9); // default-emoji BMP 2 + 1 + 6
        // VS16 is handled as zero-width (defensive; no phase emoji uses it).
        assert_eq!(display_width("x\u{FE0F}"), 1);
    }

    /// Re-derives the expected cell width with the same FE0F/ascii/wide model
    /// as `display_width`, but separately — so the alignment assertion below
    /// catches a typo/regression in `display_width` (it would still miss a flaw
    /// in the shared model itself, which the emoji set is curated to avoid).
    fn cells(s: &str) -> usize {
        s.chars()
            .map(|c| {
                if c == '\u{FE0F}' {
                    0
                } else if c.is_ascii() {
                    1
                } else {
                    2
                }
            })
            .sum()
    }

    #[test]
    fn detailed_breakdown_aligns_description_column_across_rows() {
        // Mix an emoji-prefixed phase with a bare-ASCII phase of a different
        // name length, so codepoint-count and cell-count diverge between rows
        // — the exact case the old `chars().count()` padding misaligned. The
        // description column (after the fixed-width duration field) must land
        // at the same display cell on every row.
        let out = render_phase_breakdown(
            &[phase("setup", "🔧", 2), phase("compute_digests", "", 1)],
            TimingMode::Detailed,
            false,
            true,
        );
        let desc_cols: Vec<usize> = out
            .lines()
            .filter(|l| !l.is_empty())
            .map(|l| {
                // Description is the last column; it starts after the final
                // run of 2+ spaces separating it from the duration field.
                let gap = l.rfind("  ").unwrap();
                cells(&l[..gap + 2])
            })
            .collect();
        assert_eq!(desc_cols.len(), 2);
        assert_eq!(
            desc_cols[0], desc_cols[1],
            "description columns misaligned: {out:?}"
        );
    }

    /// A user-global config outside the project evaluates under its own
    /// module scope: its relative `load()` resolves within the user config
    /// dir, and because it runs last, its writes win over the project config.
    #[test]
    fn user_scoped_config_loads_locally_and_wins_over_project_config() {
        let proj = tempfile::tempdir().unwrap();
        let user_dir = tempfile::tempdir().unwrap();

        let script = proj.path().join("hello.axl");
        std::fs::write(
            &script,
            r#"
def _impl(ctx):
    pass

hello = task(
    implementation = _impl,
    args = {"greeting": args.string(default = "default")},
)
"#,
        )
        .unwrap();
        let proj_config = proj.path().join("config.axl");
        std::fs::write(
            &proj_config,
            r#"
def config(ctx):
    ctx.tasks["hello"].args.greeting = "project"
"#,
        )
        .unwrap();

        let user_config = user_dir.path().join("config.axl");
        std::fs::write(
            user_dir.path().join("greeting.axl"),
            r#"USER_GREETING = "user""#,
        )
        .unwrap();
        std::fs::write(
            &user_config,
            r#"
load("./greeting.axl", "USER_GREETING")

def config(ctx):
    ctx.tasks["hello"].args.greeting = USER_GREETING
"#,
        )
        .unwrap();

        let rt = tokio::runtime::Runtime::new().unwrap();
        let _g = rt.enter();

        let modules: Vec<Mod> = vec![];
        let root_mod = Mod::new(
            proj.path().to_path_buf(),
            "_root".to_string(),
            proj.path().to_path_buf(),
        );
        let user_mod = Mod::user_config_scope(user_dir.path().to_path_buf());

        ModuleEnv::with(|env| -> anyhow::Result<()> {
            let loader = Loader::new(
                "test".to_string(),
                proj.path().to_path_buf(),
                proj.path().to_path_buf(),
                None,
                &modules,
            );
            let mut mpe = MultiPhaseEval::new(env, &loader);
            assert!(mpe.eval(&[script], &root_mod, &modules).is_empty());
            assert!(
                mpe.execute_configs(&[
                    (proj_config.as_path(), &root_mod),
                    (user_config.as_path(), &user_mod),
                ])
                .is_empty()
            );

            let tasks = mpe.tasks();
            assert_eq!(tasks.len(), 1);
            let overrides = tasks[0]
                .overrides()
                .downcast_ref::<Arguments>()
                .expect("overrides is an Arguments");
            assert_eq!(
                overrides
                    .get("greeting")
                    .expect("greeting override set")
                    .to_repr(),
                r#""user""#
            );
            Ok(())
        })
        .unwrap();
    }

    /// Runs a project config that loads `GREETING` through `facade.axl` and
    /// returns the config errors. `facade.axl` itself loads `GREETING` from
    /// `greeting.axl`; `facade` is the rest of its source.
    fn config_errors_loading_through(facade: &str) -> Vec<String> {
        let proj = tempfile::tempdir().unwrap();
        let script = proj.path().join("hello.axl");
        std::fs::write(
            &script,
            "def _impl(ctx):\n    pass\n\nhello = task(implementation = _impl, args = {})\n",
        )
        .unwrap();
        std::fs::write(proj.path().join("greeting.axl"), r#"GREETING = "hi""#).unwrap();
        std::fs::write(proj.path().join("facade.axl"), facade).unwrap();
        let config = proj.path().join("config.axl");
        std::fs::write(
            &config,
            "load(\"./facade.axl\", \"GREETING\")\n\ndef config(ctx):\n    print(GREETING)\n",
        )
        .unwrap();

        let rt = tokio::runtime::Runtime::new().unwrap();
        let _g = rt.enter();
        let modules: Vec<Mod> = vec![];
        let root_mod = Mod::new(
            proj.path().to_path_buf(),
            "_root".to_string(),
            proj.path().to_path_buf(),
        );
        ModuleEnv::with(|env| -> anyhow::Result<Vec<String>> {
            let loader = Loader::new(
                "test".to_string(),
                proj.path().to_path_buf(),
                proj.path().to_path_buf(),
                None,
                &modules,
            );
            let mut mpe = MultiPhaseEval::new(env, &loader);
            assert!(mpe.eval(&[script], &root_mod, &modules).is_empty());
            let failures = mpe.execute_configs(&[(config.as_path(), &root_mod)]);
            let mut errors: Vec<String> = failures.files.iter().map(|f| format!("{f:?}")).collect();
            errors.extend(failures.ended.iter().map(|e| format!("{e:?}")));
            Ok(errors)
        })
        .unwrap()
    }

    /// A `load` binds privately, as in Bazel and Buck2: a name a file only
    /// loads cannot be loaded from it.
    #[test]
    fn a_loaded_name_is_not_reexported() {
        let errors = config_errors_loading_through(r#"load("./greeting.axl", "GREETING")"#);
        assert!(
            errors.iter().any(|e| e.contains("GREETING")),
            "expected a load error naming GREETING, got {errors:?}"
        );
    }

    /// Re-exporting takes an explicit top-level binding of a private alias.
    #[test]
    fn an_explicit_binding_reexports_a_loaded_name() {
        let errors = config_errors_loading_through(
            "load(\"./greeting.axl\", _GREETING = \"GREETING\")\n\nGREETING = _GREETING\n",
        );
        assert!(errors.is_empty(), "{errors:?}");
    }
}

pub struct FinishedEval;
