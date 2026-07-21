use std::io::IsTerminal;
use std::path::{Path, PathBuf};

use anyhow::anyhow;
use starlark::environment::{FrozenModule, Module};
use starlark::eval::Evaluator;
use starlark::values::{Heap, Value, ValueLike};
use uuid::Uuid;

use crate::banner;
use crate::ci::on_recognized_ci;
use crate::engine::arguments::Arguments;
use crate::engine::bazel::Bazel;
use crate::engine::config_context::ConfigContext;
use crate::engine::feature::{Feature, FeatureLike, FrozenFeature};
use crate::engine::feature_context::FeatureContext;
use crate::engine::feature_map::FeatureMap;
use crate::engine::task::{FrozenTask, Task, TaskLike};
use crate::engine::task_context::TaskContext;
use crate::engine::task_info::PhaseRecord;
use crate::engine::task_info::TaskInfo;
use crate::engine::task_map::TaskMap;
use crate::engine::telemetry::{self, ExporterSpec, Telemetry};
use crate::engine::r#trait::extract_trait_type_id;
use crate::engine::trait_map::TraitMap;
use crate::eval::error::EvalError;
use crate::eval::load::AxlLoader;
use crate::eval::task::FrozenTaskModuleLike;
use crate::module::Mod;

// ANSI SGR parameters for the closing bookend.
//
// Terminal-state colors are fixed semaphores (green/yellow/red). The
// opening "Running" line uses the configurable highlight color from
// `ASPECT_CLI_HIGHLIGHT_COLOR` (default `_DEFAULT_HIGHLIGHT_COLOR`),
// mirroring AXL's `highlight_style` for mid-task phase updates.
const SGR_BOLD_GREEN: &str = "1;32";
const SGR_BOLD_YELLOW: &str = "1;33";
const SGR_BOLD_RED: &str = "1;31";
const SGR_RESET: &str = "\x1b[0m";

/// Default highlight color for the opening "→ 🎬 Running" line when
/// `ASPECT_CLI_HIGHLIGHT_COLOR` is unset. `1;36` is bold cyan — reads
/// cleanly on dark terminals. Common overrides: `94` (bright blue),
/// `38;5;75` (256-color sky blue), or empty to disable.
const DEFAULT_HIGHLIGHT_COLOR: &str = "1;36";
const HIGHLIGHT_COLOR_ENV: &str = "ASPECT_CLI_HIGHLIGHT_COLOR";

/// Return `(verb_color_prefix, reset)` for the "Running" verb on the
/// task-starting line. `verb_color_prefix` is empty when color is
/// disabled or `ASPECT_CLI_HIGHLIGHT_COLOR=""`; `reset` is empty when
/// color is disabled. Color is enabled when stderr is a TTY or we're on
/// a recognized CI host.
fn running_verb_color() -> (String, &'static str) {
    let color = std::io::stderr().is_terminal() || on_recognized_ci();
    let highlight = std::env::var(HIGHLIGHT_COLOR_ENV)
        .ok()
        .unwrap_or_else(|| DEFAULT_HIGHLIGHT_COLOR.to_string());
    let verb_seq = if color && !highlight.is_empty() {
        format!("\x1b[{}m", highlight)
    } else {
        String::new()
    };
    let reset = if color { SGR_RESET } else { "" };
    (verb_seq, reset)
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
/// 1. `eval`              — load task and feature scripts; inserts discovered
///                          `Task` and `Feature` values into `self.tasks` /
///                          `self.features` on the shared heap.
/// 2. `eval_config`       — run config.axl files; mutations visible via shared heap.
/// 3. `eval_feature_impls`— run enabled feature implementations.
/// 4. `execute_with_args` — call the selected task implementation.
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
    /// Global trait map allocated on the shared heap during Phase 2.
    trait_map_value: Option<Value<'v>>,
    /// Telemetry handle allocated on the heap and shared between
    /// ConfigContext and FeatureContext as `ctx.telemetry`. The runtime
    /// drains exporter specs out of it (via `drain_exporters`) after phase 3.
    telemetry_value: Value<'v>,
}

impl<'v, 'l> MultiPhaseEval<'v, 'l> {
    pub fn new(env: &'l ModuleEnv<'v>, loader: &'l AxlLoader<'l>) -> Self {
        let heap = env.heap();
        let telemetry_value = Telemetry::alloc(heap);
        MultiPhaseEval {
            env,
            loader,
            tasks: heap.alloc(TaskMap::new()),
            features: heap.alloc(FeatureMap::new()),
            trait_map_value: None,
            telemetry_value,
        }
    }

    /// Drain exporter specs collected from `ctx.telemetry.exporters.add(...)`
    /// during phases 2-3. Intended to be called once, after
    /// `execute_features_with_args` completes and before phase 4 begins.
    pub fn drain_exporters(&self) -> Vec<ExporterSpec> {
        telemetry::drain_exporters(self.telemetry_value)
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
    #[tracing::instrument(skip_all)]
    pub fn eval(
        &mut self,
        scripts: &[PathBuf],
        root_mod: &'l Mod,
        modules: &'l Vec<Mod>,
    ) -> Result<(), EvalError> {
        let heap = self.heap();
        let task_map = self
            .tasks
            .downcast_ref::<TaskMap>()
            .expect("self.tasks is a TaskMap");
        let feature_map = self
            .features
            .downcast_ref::<FeatureMap>()
            .expect("self.features is a FeatureMap");

        // Evaluate auto-discovered AXL scripts (axl_sources in repo root)
        for path in scripts {
            let frozen = self.eval_file(&root_mod, path)?;
            for symbol in frozen.tasks() {
                let owned = frozen
                    .get(symbol.as_str())
                    .map_err(|_| EvalError::MissingSymbol(symbol.clone()))?;
                let frozen_value = owned
                    .value()
                    .unpack_frozen()
                    .expect("value from FrozenModule is always frozen");
                heap.access_owned_frozen_value(&owned);
                let live_task = Task::from_frozen(frozen_value, heap);
                task_map.insert(heap.alloc(live_task));
            }
        }

        // Evaluate module specs (use_task and use_feature entries from MODULE.aspect)
        for mode in modules.iter().chain(vec![root_mod]) {
            for (abs_path, (label, symbols)) in mode.tasks.iter() {
                let frozen = self.eval_file(&mode, &abs_path)?;
                for symbol in symbols {
                    if !frozen.has_name(symbol) {
                        return Err(EvalError::UnknownError(anyhow!(
                            "task symbol {:?} not found in @{} module use_task({:?})",
                            symbol,
                            mode.name,
                            label
                        )));
                    }
                    if !frozen.has_task(symbol) {
                        return Err(EvalError::UnknownError(anyhow!(
                            "invalid use_task({:?}, {:?}) in @{} module",
                            label,
                            symbol,
                            mode.name
                        )));
                    }
                    let owned = frozen
                        .get(symbol)
                        .map_err(|_| EvalError::MissingSymbol(symbol.clone()))?;
                    let frozen_value = owned
                        .value()
                        .unpack_frozen()
                        .expect("value from FrozenModule is always frozen");
                    heap.access_owned_frozen_value(&owned);
                    let live_task = Task::from_frozen(frozen_value, heap);
                    task_map.insert(heap.alloc(live_task));
                }
            }

            for (abs_path, symbol) in mode.features.iter() {
                let frozen = self.eval_file(&mode, &abs_path)?;
                let owned = frozen
                    .get(symbol.as_str())
                    .map_err(|_| EvalError::MissingSymbol(symbol.clone()))?;
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
            }
        }

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
    /// A `TraitMap` is allocated on the shared heap. Config functions mutate `Task`
    /// and `Feature` values in-place via `set_attr` — the same `Value<'v>` objects
    /// discovered in Phase 1 are reused, so subsequent phases see the updated state.
    /// `ctx.tasks.add(...)` may insert additional tasks into `self.tasks`.
    ///
    /// `configs` are absolute paths evaluated in order, so a later file's writes
    /// win over an earlier one's. Paths need not live under `mode.root` — the
    /// user-global `~/.aspect/config.axl` is passed here as the final entry.
    #[tracing::instrument(name = "execute.configs", skip_all)]
    pub fn execute_configs(&mut self, configs: &[PathBuf], mode: &'l Mod) -> Result<(), EvalError> {
        let heap = self.heap();

        // Register trait types from all tasks into a TraitMap; instances are created lazily
        // on first access (from config.axl or the task implementation).
        let trait_map = TraitMap::new();
        for task_val in self
            .tasks
            .downcast_ref::<TaskMap>()
            .expect("self.tasks is a TaskMap")
            .values()
        {
            if let Some(task) = task_val.downcast_ref::<Task>() {
                for trait_value in task.traits() {
                    if let Some(id) = extract_trait_type_id(*trait_value) {
                        trait_map.insert(id, *trait_value);
                    }
                }
            }
        }
        let trait_map_value = heap.alloc(trait_map);
        self.trait_map_value = Some(trait_map_value);

        let context_value = heap.alloc(ConfigContext::new(
            self.tasks,
            self.trait_map_value.unwrap(),
            self.features,
            self.telemetry_value,
        ));

        for config_path in configs {
            let frozen = self.eval_file(mode, config_path)?;

            let function_name = "config";
            let def = frozen
                .get(function_name)
                .map_err(|_| EvalError::MissingSymbol(function_name.to_string()))?;
            let func = heap.access_owned_frozen_value(&def);

            let mut eval = Evaluator::new(&self.env.0);
            eval.set_loader(self.loader);
            eval.extra = Some(&self.loader.env);
            eval.eval_function(func, &[context_value], &[])?;
        }

        Ok(())
    }

    /// Print the universal "task starting" announcement — the `→ 🎬 Running …`
    /// line, or `--- :aspect: Running …` BK section header — for the task at
    /// `task_index` in `self.tasks()`.
    ///
    /// Called from the CLI before `execute_features_with_args` so that any
    /// diagnostic output emitted during feature initialization (auth WARNINGs,
    /// tip blocks, etc.) is framed by the task header. Without this, the header
    /// would only appear after features finish — and feature-init output would
    /// look like it preceded any task running.
    pub fn print_running_task_header(
        &self,
        task_index: usize,
        task_name: &str,
        task_name_meaningful: bool,
    ) -> Result<(), EvalError> {
        let task = *self.tasks().get(task_index).ok_or_else(|| {
            EvalError::UnknownError(anyhow!("task index {} out of range", task_index))
        })?;
        let (verb_seq, reset) = running_verb_color();
        let label = task_label(task.group(), &task.kind(), task_name, task_name_meaningful);
        // Identity banner above the task header — see `crate::banner`.
        if banner::show_runtime_banner() {
            eprintln!("{}\n", banner::line_from_pkg());
        }
        if std::env::var_os("BUILDKITE").is_some() {
            // The BK section header replaces the `→` line on BK (avoids
            // duplicating the same text in the BK log viewer).
            eprintln!("--- :aspect: {}Running{} {}", verb_seq, reset, label);
        } else {
            // 🎬 (clapper board) pairs with the closing ✅ / ⚠️ / ❌ as
            // the task's bookend.
            eprintln!("→ 🎬 {}Running{} {}", verb_seq, reset, label);
        }
        Ok(())
    }

    /// Phase 3: run enabled feature implementations.
    ///
    /// Must be called after `eval_config` so that config files have had the opportunity
    /// to enable or disable features. `args_builder` builds the fully-merged `Arguments`
    /// for each feature (callers handle CLI/override/default precedence). Features whose
    /// resolved `enabled` is `False` are skipped.
    #[tracing::instrument(name = "execute.features", skip_all)]
    pub fn execute_features_with_args(
        &mut self,
        args_builder: impl Fn(&dyn FeatureLike<'v>, Heap<'v>) -> Arguments<'v>,
    ) -> Result<(), EvalError> {
        let heap = self.heap();

        let trait_map_value = self.trait_map_value.ok_or_else(|| {
            EvalError::UnknownError(anyhow!(
                "eval_config() must be called before execute_features_with_args()"
            ))
        })?;

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
            ));
            let mut eval = Evaluator::new(&self.env.0);
            eval.set_loader(self.loader);
            eval.extra = Some(&self.loader.env);
            eval.eval_function(feature.implementation(), &[fctx], &[])
                .map_err(|e| {
                    EvalError::UnknownError(anyhow!("Feature implementation failed: {:?}", e))
                })?;
        }

        Ok(())
    }

    /// Phase 4: execute the task at the given index with pre-built args.
    ///
    /// The index refers to the position in `self.tasks()` (same Vec returned to
    /// the CLI when building the command tree). `args_builder` returns the
    /// fully-merged `Arguments` (callers handle CLI/override/default precedence).
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
        args_builder: impl FnOnce(&dyn TaskLike<'v>, Heap<'v>) -> Arguments<'v>,
    ) -> Result<Option<u8>, EvalError> {
        let task = *self.tasks().get(task_index).ok_or_else(|| {
            EvalError::UnknownError(anyhow!("task index {} out of range", task_index))
        })?;

        let heap = self.heap();
        let task_id = task_id.unwrap_or_else(|| Uuid::new_v4().to_string());
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

        // The opening `→ 🎬 Running …` (or BK `--- :aspect: Running …`)
        // header is printed by `print_running_task_header` BEFORE phase
        // 3 (feature impls) so that any diagnostic output emitted during
        // feature initialization is framed by the header. Only the
        // closing announcement is built here; phase boundaries inside
        // the task body are still announced from AXL via
        // `lib/lifecycle.axl::announce`. Color setup mirrors the helper
        // so the verdict glyph and labels match.
        let color = std::io::stderr().is_terminal() || on_recognized_ci();
        let sgr = |params: &str| {
            if color {
                format!("\x1b[{}m", params)
            } else {
                String::new()
            }
        };
        let bold_green = sgr(SGR_BOLD_GREEN);
        let bold_yellow = sgr(SGR_BOLD_YELLOW);
        let bold_red = sgr(SGR_BOLD_RED);
        let reset = if color { SGR_RESET } else { "" };

        let task_trait_map = match self.trait_map_value {
            Some(tmap_val) => {
                if let Some(tmap) = tmap_val.downcast_ref::<TraitMap>() {
                    tmap.scoped(&task.trait_type_ids(), heap)
                } else {
                    heap.alloc(TraitMap::new())
                }
            }
            None => heap.alloc(TraitMap::new()),
        };

        let bazel = heap.alloc(Bazel {
            active_rc: std::cell::RefCell::new(None),
        });
        // Allocate `task_info` on the heap and keep the resulting Value
        // so we can read back timing + phases after `_impl` returns.
        // `TaskInfo::started_at` is the authoritative task start (stamped
        // in `TaskInfo::new`), so we don't need a stack-local `task_start`.
        let task_info_val = heap.alloc(task_info);
        let context = heap.alloc(TaskContext::new(
            heap.alloc(task_args),
            task_trait_map,
            task_info_val,
            bazel,
        ));

        let mut eval = Evaluator::new(&self.env.0);
        eval.set_loader(self.loader);
        eval.extra = Some(&self.loader.env);

        let impl_result = eval.eval_function(task.implementation(), &[context], &[]);
        run_deferred(context, &mut eval);
        let ret = impl_result?;
        let (exit_code, flagged, conclusion) = unpack_task_return(ret);

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
        let breakdown = render_phase_breakdown(&phases, timing, failed);
        let timing_segment = render_timing_segment(timing, elapsed);
        let conclusion_suffix = if conclusion.is_empty() {
            String::new()
        } else {
            format!(" · {}", conclusion)
        };
        let exit_suffix = match exit_code {
            Some(code) if code != 0 => format!(" (exit code {})", code),
            _ => String::new(),
        };
        let on_bk = std::env::var_os("BUILDKITE")
            .map(|v| !v.is_empty())
            .unwrap_or(false);
        let label = task_label(&task_group, &task_kind, &task_name, task_name_meaningful);
        let verdict = Verdict::pick(exit_code, flagged, &bold_green, &bold_yellow, &bold_red);
        if on_bk {
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
            // failing phase and the summary stay expanded. On a clean run we
            // keep `---` and let the auto-expand handle the summary.
            let unclean = failed || flagged;
            if unclean {
                eprintln!("^^^ +++");
            }
            let bookend_marker = if unclean { "+++" } else { "---" };
            eprintln!(
                "{} {} {}{}{} · {}{}{}{}{}",
                bookend_marker,
                verdict.bk_shortcode,
                verdict.color,
                verdict.verb,
                reset,
                label,
                exit_suffix,
                timing_segment,
                conclusion_suffix,
                breakdown,
            );
        } else {
            eprintln!();
            eprintln!(
                "→ {} {}{}{} {}{}{}{}{}",
                verdict.glyph,
                verdict.color,
                verdict.verb,
                reset,
                label,
                exit_suffix,
                timing_segment,
                conclusion_suffix,
                breakdown,
            );
        }

        Ok(exit_code)
    }

    pub fn finish(self) -> FinishedEval {
        FinishedEval
    }
}

fn run_deferred<'v>(context: Value<'v>, eval: &mut Evaluator<'v, '_, '_>) {
    let Some(ctx) = context.downcast_ref::<TaskContext>() else {
        return;
    };
    let defers = ctx.drain_defers();
    for defer in defers {
        if let Err(e) = eval.eval_function(defer.callable, &defer.args, &defer.kwargs) {
            tracing::error!(error = %e, "ctx.defer callable failed");
        }
    }
}

/// Three-way verdict rendered on the closing bookend line. Selected
/// from the task's exit code and `flagged` bit:
///
///   exit 0, !flagged → ✅ Passed   (bold green)
///   exit 0,  flagged → ⚠️ Flagged  (bold yellow)
///   exit non-0       → ❌ Failed   (bold red)
///
/// Off Buildkite the runtime emits `→ <glyph> <verb> …`; on BK it
/// emits `<marker> <bk_shortcode> <verb> · …` so the closing line lands
/// as its own collapsible section header — `+++` (expanded) on a
/// failed/flagged verdict so the timing summary stays open alongside the
/// expanded failing phase, else `---` (collapsed, auto-expanded by BK as
/// the last group).
struct Verdict<'a> {
    /// BK emoji shortcode (e.g. `:white_check_mark:`); used in the
    /// `--- :<shortcode>: <verb> …` BK section header.
    bk_shortcode: &'a str,
    /// Unicode glyph (e.g. `✅`); used off Buildkite. `⚠️` carries a
    /// trailing space — it pre-dates the emoji standard and renders
    /// 1-cell-wide in some terminals where ✅/❌ are reliably 2-cell.
    /// The space keeps the line aligned across verdicts.
    glyph: &'a str,
    verb: &'a str,
    /// ANSI color prefix (e.g. `bold_green`). Empty when `color` is
    /// off (`!is_terminal() && !on_ci`). Paired with `reset` at the
    /// call site.
    color: &'a str,
}

impl<'a> Verdict<'a> {
    fn pick(
        exit_code: Option<u8>,
        flagged: bool,
        bold_green: &'a str,
        bold_yellow: &'a str,
        bold_red: &'a str,
    ) -> Self {
        match (exit_code, flagged) {
            (Some(0) | None, false) => Self {
                bk_shortcode: ":white_check_mark:",
                glyph: "✅",
                verb: "Passed",
                color: bold_green,
            },
            (Some(0) | None, true) => Self {
                bk_shortcode: ":warning:",
                glyph: "⚠️ ",
                verb: "Flagged",
                color: bold_yellow,
            },
            (Some(_), _) => Self {
                bk_shortcode: ":x:",
                glyph: "❌",
                verb: "Failed",
                color: bold_red,
            },
        }
    }
}

/// Unpack the return value of `_impl` into `(exit_code, flagged, conclusion)`.
///
/// Tasks may return either a bare `int` (treated as `TaskConclusion(
/// exit_code=int)`) or a `TaskConclusion` record carrying the full
/// terminal state. Anything else yields `(None, false, "")` — the
/// runtime renders that as `✅ Passed` with no conclusion suffix.
fn unpack_task_return<'v>(ret: starlark::values::Value<'v>) -> (Option<u8>, bool, String) {
    if let Some(tc) = ret.downcast_ref::<crate::engine::task_info::TaskConclusion>() {
        (Some(tc.exit_code as u8), tc.flagged, tc.text.clone())
    } else if let Some(code) = ret.unpack_i32() {
        (Some(code as u8), false, String::new())
    } else {
        (None, false, String::new())
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
/// "→ ✅ Passed" / "→ ⚠️ Flagged" / "→ ❌ Failed" line. Driven by the
/// top-level `--task:timing-summary` CLI flag.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TimingMode {
    /// No timing summary at all — not even the total.
    /// `→ ✅ Passed `lint` task`
    None,
    /// Total only.
    /// `→ ✅ Passed `lint` task in 46.8s`
    Total,
    /// Total + inline phase breakdown.
    /// `→ ✅ Passed `lint` task in 46.8s — 🌱 Init 0.2s · 🔍 Detect 1.2s · ...`
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
                "invalid timing mode {:?}; expected one of: none, total, short, detailed",
                other
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

/// Render the phase breakdown that trails the runtime's "→ Completed"
/// / "→ Failed" line.
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
fn render_phase_breakdown(phases: &[PhaseRecord], mode: TimingMode, failed: bool) -> String {
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
    // Prefix the caller-supplied `Phase(emoji=...)` when present; an empty
    // emoji stays bare. The Detailed branch aligns columns by `display_width`
    // (which counts the emoji as two cells); Short mode has no columns.
    let phase_label = |p: &PhaseRecord| -> String {
        let name = if p.display_name.is_empty() {
            titlecase(&p.name)
        } else {
            p.display_name.clone()
        };
        if p.emoji.is_empty() {
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
        TimingMode, display_width, format_duration, render_phase_breakdown, render_timing_segment,
        task_label_for,
    };
    use crate::engine::task_info::PhaseRecord;
    use std::time::Duration;

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
        assert_eq!(render_phase_breakdown(&phases, TimingMode::None, false), "");
        assert_eq!(
            render_phase_breakdown(&phases, TimingMode::Total, false),
            ""
        );
    }

    #[test]
    fn phase_breakdown_short_is_inline() {
        let phases = [phase("setup", "🔧", 2), phase("build", "🔨", 3)];
        let out = render_phase_breakdown(&phases, TimingMode::Short, false);
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
}

pub struct FinishedEval;
