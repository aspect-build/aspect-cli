use std::cell::RefCell;
use std::time::Duration;
use std::time::Instant;

use allocative::Allocative;
use derive_more::Display;
use starlark::environment::Methods;
use starlark::environment::MethodsBuilder;
use starlark::environment::MethodsStatic;
use starlark::starlark_module;
use starlark::starlark_simple_value;
use starlark::values::AllocValue;
use starlark::values::Heap;
use starlark::values::NoSerialize;
use starlark::values::ProvidesStaticType;
use starlark::values::StarlarkValue;
use starlark::values::Trace;
use starlark::values::Tracer;
use starlark::values::Value;
use starlark::values::ValueLike;
use starlark::values::list::AllocList;
use starlark::values::none::NoneOr;
use starlark::values::none::NoneType;
use starlark::values::starlark_value;

/// One closed phase entry in the task's phase log.
///
/// `interrupted` is `true` only for the entry created by
/// `close_active_phase()` when the runtime auto-closes the still-open
/// phase at task exit. Renderers use it (in combination with the
/// task's terminal status) to render a `(failed after <duration>)`
/// suffix on the phase that was active when the task failed, instead
/// of a bare duration. Phases closed by an explicit `phase()`
/// transition leave `interrupted = false`.
///
/// `emoji` is an optional per-phase decoration string. When the
/// caller passed `emoji=...` to `ctx.task.phase(...)`, it lands here
/// verbatim; rendering surfaces (BK section headers, Task timing
/// table, CLI phase entry) read it via the per-entry dict from
/// `phases()`. Empty string when no emoji was supplied.
///
/// `display_name` is an optional caller-supplied override for the
/// phase's display label. When non-empty, renderers use it verbatim;
/// when empty, they titlecase `name` themselves (`build` → `Build`,
/// `bazel_query` → `Bazel query`). The only standard phase that needs
/// this is `preflight` → `Pre-flight` (naive titlecase produces
/// `Preflight` which reads oddly).
#[derive(Debug, Clone)]
pub struct PhaseRecord {
    pub name: String,
    pub description: String,
    pub duration: Duration,
    pub interrupted: bool,
    pub emoji: String,
    pub display_name: String,
}

/// The currently-active phase, if any.
#[derive(Debug, Clone)]
pub struct CurrentPhase {
    pub name: String,
    pub description: String,
    pub started_at: Instant,
    pub emoji: String,
    pub display_name: String,
}

/// Starlark-facing snapshot of one task phase, returned from
/// `ctx.task.phases()` (closed phases) and `ctx.task.current_phase()`
/// (the live phase, if any).
///
/// `duration_ms` is captured at construction time. For closed phases
/// this is the phase's fixed duration; for the live phase it's
/// `now - started_at` at the moment `current_phase()` was called.
/// Callers that want a refreshed live elapsed call `current_phase()`
/// again — `TaskPhase` values are intentionally snapshots, not live
/// views.
///
/// `interrupted` is `True` only for the entry the runtime auto-closes
/// at task exit (the phase that was active when `_impl` returned).
/// Always `False` for the live phase.
#[derive(Debug, Clone, ProvidesStaticType, Display, NoSerialize, Allocative)]
#[display("<TaskPhase {}>", name)]
pub struct TaskPhase {
    pub name: String,
    pub description: String,
    pub duration_ms: i64,
    pub interrupted: bool,
    pub emoji: String,
    pub display_name: String,
}

starlark_simple_value!(TaskPhase);

#[starlark_value(type = "TaskPhase")]
impl<'v> StarlarkValue<'v> for TaskPhase {
    fn get_methods() -> Option<&'static Methods> {
        static RES: MethodsStatic = MethodsStatic::new();
        RES.methods(task_phase_methods)
    }
}

#[starlark_module]
fn task_phase_methods(registry: &mut MethodsBuilder) {
    /// Phase id (`build`, `test`, `preflight`, `init`, …) — the
    /// lowercase identifier callers passed as `Phase(name=...)`.
    #[starlark(attribute)]
    fn name<'v>(this: Value<'v>) -> anyhow::Result<String> {
        Ok(this.downcast_ref::<TaskPhase>().unwrap().name.clone())
    }

    /// Human-readable description (e.g. `"Build delivery targets"`),
    /// or `""` if unset.
    #[starlark(attribute)]
    fn description<'v>(this: Value<'v>) -> anyhow::Result<String> {
        Ok(this
            .downcast_ref::<TaskPhase>()
            .unwrap()
            .description
            .clone())
    }

    /// Duration in milliseconds. For closed phases this is the fixed
    /// recorded duration; for the live phase it's the elapsed time
    /// at the moment `current_phase()` was called.
    #[starlark(attribute)]
    fn duration_ms<'v>(this: Value<'v>) -> anyhow::Result<i64> {
        Ok(this.downcast_ref::<TaskPhase>().unwrap().duration_ms)
    }

    /// `True` only for the phase that was active when the runtime
    /// auto-closed it at task exit. Always `False` for the live
    /// phase and for phases closed via a normal `phase()` transition.
    #[starlark(attribute)]
    fn interrupted<'v>(this: Value<'v>) -> anyhow::Result<bool> {
        Ok(this.downcast_ref::<TaskPhase>().unwrap().interrupted)
    }

    /// Producer-supplied phase icon (e.g. `"🔨"`) from the input
    /// `Phase(emoji=...)`. `""` when unset.
    #[starlark(attribute)]
    fn emoji<'v>(this: Value<'v>) -> anyhow::Result<String> {
        Ok(this.downcast_ref::<TaskPhase>().unwrap().emoji.clone())
    }

    /// Producer-supplied display-label override (e.g. `"Pre-flight"`)
    /// from the input `Phase(display_name=...)`. Renderers use this
    /// verbatim when non-empty; when empty, they titlecase `name`.
    #[starlark(attribute)]
    fn display_name<'v>(this: Value<'v>) -> anyhow::Result<String> {
        Ok(this
            .downcast_ref::<TaskPhase>()
            .unwrap()
            .display_name
            .clone())
    }
}

impl TaskPhase {
    /// Construct a `TaskPhase` snapshot from a closed `PhaseRecord`.
    pub fn from_record(r: &PhaseRecord) -> Self {
        Self {
            name: r.name.clone(),
            description: r.description.clone(),
            duration_ms: r.duration.as_millis() as i64,
            interrupted: r.interrupted,
            emoji: r.emoji.clone(),
            display_name: r.display_name.clone(),
        }
    }

    /// Construct a `TaskPhase` snapshot from the live `CurrentPhase`,
    /// computing `duration_ms` against `now`.
    pub fn from_current(c: &CurrentPhase, now: Instant) -> Self {
        Self {
            name: c.name.clone(),
            description: c.description.clone(),
            duration_ms: now.duration_since(c.started_at).as_millis() as i64,
            interrupted: false,
            emoji: c.emoji.clone(),
            display_name: c.display_name.clone(),
        }
    }
}

/// Per-invocation task metadata exposed to AXL as `ctx.task`.
///
/// In addition to identity fields (name, group, key, id), `TaskInfo`
/// owns the task's timing state and a phase log. AXL marks phase
/// boundaries via `ctx.task.phase(name, description=, emoji=,
/// display_name=)` (or via the higher-level `task_update(...,
/// phase=Phase(...))` wrapper in `lib/lifecycle.axl`); the runtime
/// reads `phases` and `started_at` after `_impl` returns to render
/// the "→ Completed" / "→ Failed" line with a phase breakdown.
///
/// A complex (`alloc_complex`) value because of the interior-mutable
/// `RefCell` fields holding `phases` and `current_phase`.
#[derive(Debug, ProvidesStaticType, Display, NoSerialize, Allocative)]
#[display("<TaskInfo>")]
pub struct TaskInfo {
    pub name: String,
    pub group: Vec<String>,
    pub task_key: String,
    pub task_id: String,

    #[allocative(skip)]
    pub started_at: Instant,
    #[allocative(skip)]
    pub phases: RefCell<Vec<PhaseRecord>>,
    #[allocative(skip)]
    pub current_phase: RefCell<Option<CurrentPhase>>,
}

impl TaskInfo {
    /// Construct a fresh TaskInfo, stamping `started_at` to now. The
    /// `started_at` field is the authoritative bookend for total task
    /// wall time and the "init" phase synthesis on first `phase()` call.
    pub fn new(name: String, group: Vec<String>, task_key: String, task_id: String) -> Self {
        Self {
            name,
            group,
            task_key,
            task_id,
            started_at: Instant::now(),
            phases: RefCell::new(Vec::new()),
            current_phase: RefCell::new(None),
        }
    }

    /// Close the active phase, if any. Idempotent — safe to call after
    /// `_impl` returns whether or not the task left a phase open.
    /// Records the entry with `interrupted = true` so renderers can
    /// distinguish "phase ended at task exit" (typically the failure
    /// phase on a non-zero exit) from "phase ended via a normal
    /// `phase()` transition".
    pub fn close_active_phase(&self) {
        let mut current = self.current_phase.borrow_mut();
        if let Some(active) = current.take() {
            self.phases.borrow_mut().push(PhaseRecord {
                name: active.name,
                description: active.description,
                duration: active.started_at.elapsed(),
                interrupted: true,
                emoji: active.emoji,
                display_name: active.display_name,
            });
        }
    }
}

unsafe impl<'v> Trace<'v> for TaskInfo {
    fn trace(&mut self, _tracer: &Tracer<'v>) {
        // No `Value<'v>` references stored — nothing to trace.
    }
}

impl<'v> AllocValue<'v> for TaskInfo {
    fn alloc_value(self, heap: Heap<'v>) -> Value<'v> {
        // `alloc_complex_no_freeze` is the right shape: TaskInfo's
        // RefCell fields aren't Sync (and don't need to be — Starlark
        // execution is single-threaded per task), and the value is
        // never frozen during its lifecycle.
        heap.alloc_complex_no_freeze(self)
    }
}

#[starlark_value(type = "TaskInfo")]
impl<'v> StarlarkValue<'v> for TaskInfo {
    fn get_methods() -> Option<&'static Methods> {
        static RES: MethodsStatic = MethodsStatic::new();
        RES.methods(task_info_methods)
    }
}

#[starlark_module]
fn task_info_methods(registry: &mut MethodsBuilder) {
    /// The name of the task.
    #[starlark(attribute)]
    fn name<'v>(this: Value<'v>) -> anyhow::Result<String> {
        let info = this
            .downcast_ref::<TaskInfo>()
            .ok_or_else(|| anyhow::anyhow!("name: receiver is not a TaskInfo"))?;
        Ok(info.name.clone())
    }

    /// The group(s) this task belongs to.
    #[starlark(attribute)]
    fn group<'v>(this: Value<'v>) -> anyhow::Result<Vec<String>> {
        let info = this
            .downcast_ref::<TaskInfo>()
            .ok_or_else(|| anyhow::anyhow!("group: receiver is not a TaskInfo"))?;
        Ok(info.group.clone())
    }

    /// A short human-readable key identifying this task invocation.
    /// Set via --task-key on the CLI, or auto-generated as a friendly name (e.g. "fluffy-parakeet").
    #[starlark(attribute)]
    fn key<'v>(this: Value<'v>) -> anyhow::Result<String> {
        let info = this
            .downcast_ref::<TaskInfo>()
            .ok_or_else(|| anyhow::anyhow!("key: receiver is not a TaskInfo"))?;
        Ok(info.task_key.clone())
    }

    /// A globally unique UUID v4 for this task invocation.
    /// Always auto-generated; use key for a short human-readable discriminator.
    #[starlark(attribute)]
    fn id<'v>(this: Value<'v>) -> anyhow::Result<String> {
        let info = this
            .downcast_ref::<TaskInfo>()
            .ok_or_else(|| anyhow::anyhow!("id: receiver is not a TaskInfo"))?;
        Ok(info.task_id.clone())
    }

    /// Task wall time so far in milliseconds (now - task spawn).
    /// Refreshed on every read; surface renderers use this for the
    /// "Task time" item in the status surface header.
    #[starlark(attribute)]
    fn elapsed_ms<'v>(this: Value<'v>) -> anyhow::Result<i64> {
        let info = this
            .downcast_ref::<TaskInfo>()
            .ok_or_else(|| anyhow::anyhow!("elapsed_ms: receiver is not a TaskInfo"))?;
        Ok(info.started_at.elapsed().as_millis() as i64)
    }

    /// Snapshot of closed phases as a `list[TaskPhase]`. The
    /// currently-active phase is NOT included — use `current_phase()`
    /// for that.
    ///
    /// Each `TaskPhase` is a snapshot (not a live view):
    /// `duration_ms`, `interrupted`, `emoji`, `display_name`, and
    /// `description` are captured at the moment `phases()` is called.
    ///
    /// Called as a method rather than an attribute because the list
    /// is freshly allocated on each call (Starlark attributes don't
    /// expose the heap).
    fn phases<'v>(this: Value<'v>, heap: Heap<'v>) -> anyhow::Result<Value<'v>> {
        let info = this
            .downcast_ref::<TaskInfo>()
            .ok_or_else(|| anyhow::anyhow!("phases: receiver is not a TaskInfo"))?;
        let phases = info.phases.borrow();
        let items: Vec<Value<'v>> = phases
            .iter()
            .map(|p| heap.alloc(TaskPhase::from_record(p)))
            .collect();
        Ok(heap.alloc(AllocList(items)))
    }

    /// The currently-active `TaskPhase`, or `None` if no phase is
    /// open (tasks that never called `task_update(..., phase=...)`
    /// see `None`).
    ///
    /// Returns a fresh snapshot per call — `duration_ms` is computed
    /// against `now`. Callers wanting a refreshed elapsed re-invoke
    /// the method.
    fn current_phase<'v>(this: Value<'v>, heap: Heap<'v>) -> anyhow::Result<NoneOr<Value<'v>>> {
        let info = this
            .downcast_ref::<TaskInfo>()
            .ok_or_else(|| anyhow::anyhow!("current_phase: receiver is not a TaskInfo"))?;
        let now = Instant::now();
        Ok(match info.current_phase.borrow().as_ref() {
            Some(c) => NoneOr::Other(heap.alloc(TaskPhase::from_current(c, now))),
            None => NoneOr::None,
        })
    }

    /// Mark a phase boundary.
    ///
    /// First call: synthesizes an "init" phase covering task spawn to
    /// now (so phase durations reconcile to total task time), then
    /// opens phase `name`.
    ///
    /// Subsequent call with a different name: closes the active phase
    /// (records its duration), opens a new one.
    ///
    /// Subsequent call with the same name (incl. the active phase): no-op.
    /// This makes it safe to pass `phase=` on bare data-refresh
    /// `task_update()` emits inside a streaming loop.
    ///
    /// `emoji` is an optional decoration string carried on the phase
    /// record. Surface renderers prefix it onto the phase label when
    /// non-empty (`🔨 Build`). Empty string when unset.
    ///
    /// `display_name` is an optional display-label override. When
    /// non-empty, renderers use it verbatim; when empty, they
    /// titlecase `name`. Use it when naive titlecasing diverges from
    /// the natural English form (`preflight` → `Pre-flight`); regular
    /// underscore-separated identifiers (`bazel_query` →
    /// `Bazel query`) don't need it.
    ///
    /// Same-name no-op calls don't update emoji or display_name on
    /// the already-open phase — set them on the call that opens the
    /// phase.
    fn phase<'v>(
        this: Value<'v>,
        #[starlark(require = pos)] name: String,
        #[starlark(default = String::new())] description: String,
        #[starlark(default = String::new())] emoji: String,
        #[starlark(default = String::new())] display_name: String,
    ) -> anyhow::Result<NoneType> {
        let this = this
            .downcast_ref::<TaskInfo>()
            .ok_or_else(|| anyhow::anyhow!("phase: receiver is not a TaskInfo"))?;
        let mut current = this.current_phase.borrow_mut();
        let mut phases = this.phases.borrow_mut();
        let now = Instant::now();

        if current.is_none() && phases.is_empty() {
            // Synthetic "init" phase covers task spawn to now —
            // Aspect runtime startup, AXL eval, trait construction,
            // and any pre-first-phase task code in `_impl`. No emoji
            // and no display-name override — renderers titlecase
            // `init` to `Init`.
            phases.push(PhaseRecord {
                name: "init".to_string(),
                description: "Aspect runtime startup".to_string(),
                duration: now.duration_since(this.started_at),
                interrupted: false,
                emoji: String::new(),
                display_name: String::new(),
            });
        } else if let Some(prev) = current.as_ref() {
            if prev.name == name {
                return Ok(NoneType);
            }
            phases.push(PhaseRecord {
                name: prev.name.clone(),
                description: prev.description.clone(),
                duration: now.duration_since(prev.started_at),
                interrupted: false,
                emoji: prev.emoji.clone(),
                display_name: prev.display_name.clone(),
            });
        }
        *current = Some(CurrentPhase {
            name,
            description,
            started_at: now,
            emoji,
            display_name,
        });
        Ok(NoneType)
    }
}

/// Terminal state returned by a task's `_impl` function. The runtime
/// reads it after `_impl` returns to render the closing bookend:
///
/// - `exit_code`  — the task's exit code (0 = pass, non-zero = fail).
/// - `text`       — short, plain-text conclusion appended to the
///                  bookend as `· <text>`. Omitted when empty.
/// - `flagged`    — when `True` and `exit_code == 0`, the bookend
///                  reads `⚠️ Flagged` instead of `✅ Passed`. Ignored
///                  on non-zero exit (failure dominates).
///
/// Tasks may instead return a bare `int` (treated as `TaskConclusion(
/// exit_code=int)`) when they have nothing more to say than the
/// numeric exit. `int` and `TaskConclusion` are the only accepted
/// return types.
#[derive(Debug, Clone, ProvidesStaticType, Display, NoSerialize, Allocative)]
#[display(
    "<TaskConclusion exit_code={} text={:?} flagged={}>",
    exit_code,
    text,
    flagged
)]
pub struct TaskConclusion {
    pub exit_code: i32,
    pub text: String,
    pub flagged: bool,
}

starlark_simple_value!(TaskConclusion);

#[starlark_value(type = "TaskConclusion")]
impl<'v> StarlarkValue<'v> for TaskConclusion {
    fn get_methods() -> Option<&'static Methods> {
        static RES: MethodsStatic = MethodsStatic::new();
        RES.methods(task_conclusion_methods)
    }
}

#[starlark_module]
fn task_conclusion_methods(registry: &mut MethodsBuilder) {
    /// The task's exit code (0 = pass, non-zero = fail).
    #[starlark(attribute)]
    fn exit_code<'v>(this: Value<'v>) -> anyhow::Result<i32> {
        Ok(this.downcast_ref::<TaskConclusion>().unwrap().exit_code)
    }

    /// Short, plain-text terminal summary appended to the bookend as
    /// `· <text>`. `""` when no conclusion was supplied.
    #[starlark(attribute)]
    fn text<'v>(this: Value<'v>) -> anyhow::Result<String> {
        Ok(this.downcast_ref::<TaskConclusion>().unwrap().text.clone())
    }

    /// `True` when this task should render as `⚠️ Flagged` instead of
    /// `✅ Passed` (only when `exit_code == 0` — ignored otherwise).
    #[starlark(attribute)]
    fn flagged<'v>(this: Value<'v>) -> anyhow::Result<bool> {
        Ok(this.downcast_ref::<TaskConclusion>().unwrap().flagged)
    }
}

#[starlark_module]
pub fn register_globals(globals: &mut starlark::environment::GlobalsBuilder) {
    /// Construct a `TaskConclusion` to return from `_impl`. Carries the
    /// task's terminal state to the runtime: exit code, optional
    /// conclusion text (rendered as `· <text>` on the bookend), and
    /// optional `flagged` flag (passing-with-warning).
    ///
    /// `_impl` may return either a bare `int` (treated as
    /// `TaskConclusion(exit_code=int)`) or a `TaskConclusion`.
    #[starlark(as_type = TaskConclusion)]
    fn TaskConclusion(
        #[starlark(require = named)] exit_code: i32,
        #[starlark(require = named, default = String::new())] text: String,
        #[starlark(require = named, default = false)] flagged: bool,
    ) -> anyhow::Result<TaskConclusion> {
        Ok(TaskConclusion {
            exit_code,
            text,
            flagged,
        })
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn return_int_runs_to_completion() {
        let exit = crate::test::eval(
            r#"
def _impl(ctx):
    return 0

Test = task(implementation = _impl)
"#,
        )
        .run_task(0)
        .expect("run_task");
        assert_eq!(exit, Some(0));
    }

    #[test]
    fn return_task_conclusion_passed() {
        let exit = crate::test::eval(
            r#"
def _impl(ctx):
    return TaskConclusion(exit_code = 0, text = "12 files processed")

Test = task(implementation = _impl)
"#,
        )
        .run_task(0)
        .expect("run_task");
        assert_eq!(exit, Some(0));
    }

    #[test]
    fn return_task_conclusion_flagged() {
        let exit = crate::test::eval(
            r#"
def _impl(ctx):
    return TaskConclusion(
        exit_code = 0,
        text = "reformatted 12 files",
        flagged = True,
    )

Test = task(implementation = _impl)
"#,
        )
        .run_task(0)
        .expect("run_task");
        assert_eq!(exit, Some(0));
    }

    #[test]
    fn return_task_conclusion_failed() {
        let exit = crate::test::eval(
            r#"
def _impl(ctx):
    return TaskConclusion(exit_code = 1, text = "3 errors, 8 warnings")

Test = task(implementation = _impl)
"#,
        )
        .run_task(0)
        .expect("run_task");
        assert_eq!(exit, Some(1));
    }

    #[test]
    fn return_task_conclusion_empty_text_with_flag() {
        let exit = crate::test::eval(
            r#"
def _impl(ctx):
    return TaskConclusion(exit_code = 0, flagged = True)

Test = task(implementation = _impl)
"#,
        )
        .run_task(0)
        .expect("run_task");
        assert_eq!(exit, Some(0));
    }

    // Sanity-check that the starlark `enum()` builtin (used by
    // `lifecycle.Status` to validate task_update's `status` arg) is
    // wired into the runtime and raises on invalid values with a
    // useful error message.
    #[test]
    fn enum_builtin_validates_values() {
        let err = crate::test::eval(
            r#"
Status = enum("running", "warning", "passed", "failed")
def _impl(ctx):
    Status("warned")  # typo — not a member; should raise
    return 0

Test = task(implementation = _impl)
"#,
        )
        .run_task(0)
        .expect_err("expected invalid enum value to raise");
        let msg = err.to_string();
        assert!(
            msg.contains("warned"),
            "error should mention the bad value, got: {msg}",
        );
    }
}
