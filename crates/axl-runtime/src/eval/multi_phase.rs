use std::io::IsTerminal;
use std::path::{Path, PathBuf};

use anyhow::anyhow;
use starlark::environment::{FrozenModule, Module};
use starlark::eval::Evaluator;
use starlark::values::list::AllocList;
use starlark::values::{Heap, Value, ValueLike};
use uuid::Uuid;

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
            let rel_path = config_path
                .strip_prefix(&mode.root)
                .map_err(|e| EvalError::UnknownError(anyhow!("Failed to strip prefix: {e}")))?
                .to_path_buf();

            let abs = mode.root.join(&rel_path);
            let frozen = self.eval_file(mode, &abs)?;

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
            task_key = %task_key,
            task = tracing::field::Empty,
            task_id = tracing::field::Empty,
            exit_code = tracing::field::Empty,
        )
    )]
    pub fn execute_tasks_with_args(
        &mut self,
        task_index: usize,
        task_key: String,
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
        span.record("task", task.name());
        span.record("task_id", task_id.as_str());

        // Capture name early — `task_info` is moved into TaskContext below
        // and is no longer accessible from the closing announcement after
        // the eval returns.
        let task_name = task.name();
        let task_info = TaskInfo::new(task.name(), task.group().clone(), task_key, task_id);

        // Universal "task starting" announcement. Fires for every task —
        // built-in or custom — so users can see in CI logs which task is
        // running and what key/id it was invoked under, without each
        // task's AXL impl having to remember to print this itself. Phase
        // boundaries inside the task body are still announced from AXL
        // via `lib/lifecycle.axl::announce`; this line just frames each
        // task invocation.
        //
        // Diagnostic output → stderr. stdout is reserved for the primary
        // task output (anything a downstream consumer might want to
        // capture or pipe).
        //
        // On Buildkite, a `--- :aspect: Running …` section header
        // opens a collapsible section that groups the task's output
        // under one header — this REPLACES the `→ Running …` line on
        // BK (avoids printing the same text twice on the BK log
        // viewer). Off BK, the `→ Running …` line is the only marker.
        // Bracket form `[key]` matches BazelDefaults' "Running bazel
        // <cmd> [<key>] <targets>" so the two adjacent log lines read
        // as a pair.
        //
        // The `[<key>]` bracket is only included on CI. Locally most
        // invocations don't pass `--task-key`, so the auto-generated
        // name (e.g. `taboo-rub`) just clutters every task line. CI
        // shows it because that's where users correlate task-key with
        // the GHSC check-run / BK annotation context. Detected via
        // any of the major CI env markers.
        let on_buildkite = std::env::var_os("BUILDKITE").is_some();
        let on_ci = on_buildkite
            || std::env::var_os("CI").is_some()
            || std::env::var_os("GITHUB_ACTIONS").is_some()
            || std::env::var_os("CIRCLECI").is_some()
            || std::env::var_os("GITLAB_CI").is_some();
        let key_suffix = if on_ci {
            format!(" [{}]", task_info.task_key)
        } else {
            String::new()
        };
        // Color when stderr is a TTY or we're on a known CI host (CI
        // log viewers render ANSI even when stderr is piped).
        // `HIGHLIGHT_COLOR_ENV` overrides the highlight color used for
        // the opening "Running" verb; defaults to `DEFAULT_HIGHLIGHT_COLOR`.
        let color = std::io::stderr().is_terminal() || on_ci;
        let highlight_color = std::env::var(HIGHLIGHT_COLOR_ENV)
            .ok()
            .unwrap_or_else(|| DEFAULT_HIGHLIGHT_COLOR.to_string());
        let verb_seq = if color && !highlight_color.is_empty() {
            format!("\x1b[{}m", highlight_color)
        } else {
            String::new()
        };
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
        if on_buildkite {
            // The BK section header carries the same text the `→` line
            // would; skip the duplicate `→` print on BK.
            eprintln!(
                "--- :aspect: {}Running{} `{}` task{}",
                verb_seq, reset, task_info.name, key_suffix
            );
        } else {
            // 🎬 (clapper board) pairs with the closing ✅ / ⚠️ / ❌
            // as the task's bookend.
            eprintln!(
                "→ 🎬 {}Running{} `{}` task{}",
                verb_seq, reset, task_info.name, key_suffix
            );
        }

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

        let startup_flags = heap.alloc(AllocList([] as [String; 0]));
        let bazel = heap.alloc(Bazel { startup_flags });
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

        let ret = eval.eval_function(task.implementation(), &[context], &[])?;
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

        let duration = format_duration(elapsed);
        let failed = matches!(exit_code, Some(code) if code != 0);
        let breakdown = render_phase_breakdown(&phases, timing, failed);
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
        let verdict = Verdict::pick(exit_code, flagged, &bold_green, &bold_yellow, &bold_red);
        if on_bk {
            eprintln!(
                "--- {} {}{}{} · `{}` task{} in {}{}{}",
                verdict.bk_shortcode,
                verdict.color,
                verdict.verb,
                reset,
                task_name,
                exit_suffix,
                duration,
                conclusion_suffix,
                breakdown,
            );
        } else {
            eprintln!();
            eprintln!(
                "→ {} {}{}{} `{}` task{} in {}{}{}",
                verdict.glyph,
                verdict.color,
                verdict.verb,
                reset,
                task_name,
                exit_suffix,
                duration,
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

/// Three-way verdict rendered on the closing bookend line. Selected
/// from the task's exit code and `flagged` bit:
///
///   exit 0, !flagged → ✅ Passed   (bold green)
///   exit 0,  flagged → ⚠️ Flagged  (bold yellow)
///   exit non-0       → ❌ Failed   (bold red)
///
/// Off Buildkite the runtime emits `→ <glyph> <verb> …`; on BK it
/// emits `--- <bk_shortcode> <verb> · …` so the closing line lands as
/// its own collapsible section header.
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

/// Verbosity for the phase breakdown trailing the runtime's
/// "→ ✅ Passed" / "→ ⚠️ Flagged" / "→ ❌ Failed" line. Driven by the
/// top-level `--timing` CLI flag.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TimingMode {
    /// Total only. Same as the pre-phase output.
    /// `→ ✅ Passed `lint` task in 46.8s`
    Total,
    /// Total + inline phase breakdown.
    /// `→ ✅ Passed `lint` task in 46.8s — Init 0.2s · Detect 1.2s · ...`
    Summary,
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
            "total" => Ok(TimingMode::Total),
            "summary" => Ok(TimingMode::Summary),
            "detailed" => Ok(TimingMode::Detailed),
            other => Err(format!(
                "invalid timing mode {:?}; expected one of: total, summary, detailed",
                other
            )),
        }
    }
}

/// Render the phase breakdown that trails the runtime's "→ Completed"
/// / "→ Failed" line.
///
/// Returns `""` for `Total` mode, or for any mode when `phases` is
/// empty (task didn't opt into phases).
///
/// `Summary` returns a leading-space-and-em-dash inline form so it
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
    if phases.is_empty() || mode == TimingMode::Total {
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
    // CLI surfaces only render the emoji the caller supplied via
    // `Phase(emoji=...)`; empty stays bare. Emoji rendering on plain
    // terminals varies (some terminfos double-width them and break
    // column alignment, some render `?`), so we render bare rather
    // than inconsistently. Users who want emoji on CLI supply it
    // explicitly per phase.
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
        TimingMode::Total => unreachable!(),
        TimingMode::Summary => {
            let parts: Vec<String> = visible.iter().map(|p| format_phase(p)).collect();
            format!(" — {}", parts.join(" · "))
        }
        TimingMode::Detailed => {
            // Compute column widths so durations align. Emoji gets
            // prefixed inside the name column — names with emoji
            // render slightly wider, but the dur+desc columns still
            // line up.
            let labels: Vec<String> = visible.iter().map(|p| phase_label(p)).collect();
            let name_w = labels.iter().map(|s| s.chars().count()).max().unwrap_or(0);
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
                // Pad the label to `name_w` *characters* (not bytes) —
                // emoji sequences confuse byte-width Rust formatters.
                let pad = name_w.saturating_sub(label.chars().count());
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
/// display label (e.g. `preflight` → `Pre-flight`) pass an explicit
/// `display_name` to `ctx.task.phase(...)`; renderers prefer that
/// over this helper.
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

pub struct FinishedEval;
