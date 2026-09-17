//! Runtime-invoked hooks around a task body: `ctx.hooks.pre_task(fn)` runs
//! before `_impl`, `ctx.hooks.post_task(fn)` after it, however it ended.
//!
//! One `TaskHooks` value lives on the shared heap for the whole run, so
//! `config.axl` (phase 2), a feature impl (phase 3), and the task body
//! (phase 4) register into the same lists. The task runner in
//! `eval/multi_phase.rs` drives them:
//!
//! 1. pre-task hooks, in registration order, each called as `hook(ctx)`;
//! 2. the task body;
//! 3. post-task hooks, in registration order, each called as
//!    `hook(ctx, conclusion)` with the resolved `TaskConclusion`;
//! 4. `ctx.defer` callbacks;
//! 5. the closing bookend.
//!
//! A pre-task hook that exits or fails stands in for the body, which never
//! runs; the post-task hooks still see the resulting conclusion. A hard error
//! anywhere reaches the post-task hooks as a failed conclusion whose message
//! is the error's summary, and propagates afterwards. A post-task hook that
//! fails is reported as a warning and changes nothing.
//!
//! Registering a pre-task hook once the body has started is an error, since
//! it could never run.

use std::cell::{Cell, RefCell};

use allocative::Allocative;
use derive_more::Display;
use starlark::environment::{Methods, MethodsBuilder, MethodsStatic};
use starlark::eval::Evaluator;
use starlark::starlark_module;
use starlark::values::none::NoneType;
use starlark::values::{
    AllocValue, Freeze, FreezeError, Freezer, Heap, NoSerialize, ProvidesStaticType, StarlarkValue,
    Trace, Tracer, Value, ValueLike, starlark_value,
};

use crate::diag;
use crate::eval::TaskExit;

#[derive(Debug, ProvidesStaticType, NoSerialize, Allocative, Display)]
#[display("<TaskHooks>")]
pub struct TaskHooks<'v> {
    #[allocative(skip)]
    pre: RefCell<Vec<Value<'v>>>,
    #[allocative(skip)]
    post: RefCell<Vec<Value<'v>>>,
    #[allocative(skip)]
    started: Cell<bool>,
}

impl<'v> TaskHooks<'v> {
    pub fn alloc(heap: Heap<'v>) -> Value<'v> {
        heap.alloc(TaskHooks {
            pre: RefCell::new(Vec::new()),
            post: RefCell::new(Vec::new()),
            started: Cell::new(false),
        })
    }

    /// Mark the task as started, just before the pre-task hooks run. Pre-task
    /// registration is refused from here on: a hook registered now, even by a
    /// pre-task hook, would never run.
    pub fn start(&self) {
        self.started.set(true);
    }

    /// Run the pre-task hooks in registration order. The first error ends the
    /// sequence and stands in for the body's result.
    pub fn run_pre(
        &self,
        ctx: Value<'v>,
        eval: &mut Evaluator<'v, '_, '_>,
    ) -> Result<(), starlark::Error> {
        let hooks = self.pre.borrow().clone();
        for hook in hooks {
            eval.eval_function(hook, &[ctx], &[])?;
        }
        Ok(())
    }

    /// Run the post-task hooks in registration order with the task's
    /// conclusion. A failing hook is reported and the rest still run.
    pub fn run_post(
        &self,
        ctx: Value<'v>,
        conclusion: Value<'v>,
        eval: &mut Evaluator<'v, '_, '_>,
    ) {
        let hooks = self.post.borrow().clone();
        for hook in hooks {
            if let Err(e) = eval.eval_function(hook, &[ctx, conclusion], &[]) {
                diag::warn(&format!(
                    "post-task hook failed: {}",
                    e.without_diagnostic()
                ));
                TaskExit::debug_traceback(&e);
            }
        }
    }
}

unsafe impl<'v> Trace<'v> for TaskHooks<'v> {
    fn trace(&mut self, tracer: &Tracer<'v>) {
        for hook in self.pre.get_mut().iter_mut() {
            hook.trace(tracer);
        }
        for hook in self.post.get_mut().iter_mut() {
            hook.trace(tracer);
        }
    }
}

impl<'v> AllocValue<'v> for TaskHooks<'v> {
    fn alloc_value(self, heap: Heap<'v>) -> Value<'v> {
        heap.alloc_complex(self)
    }
}

impl<'v> Freeze for TaskHooks<'v> {
    type Frozen = FrozenTaskHooks;
    fn freeze(self, _freezer: &Freezer) -> Result<Self::Frozen, FreezeError> {
        Ok(FrozenTaskHooks)
    }
}

#[starlark_value(type = "TaskHooks")]
impl<'v> StarlarkValue<'v> for TaskHooks<'v> {
    fn get_methods() -> Option<&'static Methods> {
        static RES: MethodsStatic = MethodsStatic::new();
        RES.methods(task_hooks_methods)
    }
}

/// What a `ctx.hooks` reference becomes if a module holding one is frozen. It
/// has no run to register into, so both methods refuse; the live value keeps
/// the hooks the run will execute.
#[derive(Debug, ProvidesStaticType, NoSerialize, Allocative, Display)]
#[display("<TaskHooks>")]
pub struct FrozenTaskHooks;

starlark::starlark_simple_value!(FrozenTaskHooks);

#[starlark_value(type = "TaskHooks")]
impl<'v> StarlarkValue<'v> for FrozenTaskHooks {
    type Canonical = TaskHooks<'v>;

    fn get_methods() -> Option<&'static Methods> {
        static RES: MethodsStatic = MethodsStatic::new();
        RES.methods(task_hooks_methods)
    }
}

/// The live hooks behind `this`, or the error a frozen reference gets.
fn live<'v>(this: Value<'v>) -> starlark::Result<&'v TaskHooks<'v>> {
    this.downcast_ref::<TaskHooks>().ok_or_else(|| {
        starlark::Error::new_other(anyhow::anyhow!(
            "task hooks can no longer be registered: the module holding them is frozen"
        ))
    })
}

#[starlark_module]
fn task_hooks_methods(registry: &mut MethodsBuilder) {
    /// Run `callable(ctx)` before the task body, after every hook registered
    /// earlier. An `exit` or error here stands in for the body, which then
    /// never runs; post-task hooks still see the resulting conclusion.
    /// Refused once the body has started, so only a feature or config impl
    /// can register one.
    fn pre_task<'v>(
        this: Value<'v>,
        #[starlark(require = pos)] callable: Value<'v>,
    ) -> starlark::Result<NoneType> {
        let hooks = live(this)?;
        if hooks.started.get() {
            return Err(starlark::Error::new_other(anyhow::anyhow!(
                "pre_task: the task has already started; register pre-task hooks from \
                 config.axl or a feature, or use post_task"
            )));
        }
        hooks.pre.borrow_mut().push(callable);
        Ok(NoneType)
    }

    /// Run `callable(ctx, conclusion)` after the task body ends, however it
    /// ended: a return, an `exit`, or an error. `conclusion` is the
    /// `TaskConclusion` the runtime resolved (exit code, text, flagged,
    /// message). Post-task hooks run in registration order, before `ctx.defer`
    /// callbacks and the closing bookend. A hook that fails is reported as a
    /// warning and does not change the conclusion.
    fn post_task<'v>(
        this: Value<'v>,
        #[starlark(require = pos)] callable: Value<'v>,
    ) -> starlark::Result<NoneType> {
        live(this)?.post.borrow_mut().push(callable);
        Ok(NoneType)
    }
}

#[cfg(test)]
mod tests {
    use crate::engine::passthrough;

    /// A file the AXL under test appends one line per event to, so a test can
    /// assert on the order things ran in. Snippets reference it as `{path}`
    /// and append with `ctx.std.fs.try_append`.
    struct Trace {
        _dir: tempfile::TempDir,
        path: String,
    }

    impl Trace {
        fn new() -> Self {
            let dir = tempfile::tempdir().expect("temp dir");
            let path = dir.path().join("trace").to_string_lossy().into_owned();
            Self { _dir: dir, path }
        }

        fn read(&self) -> String {
            std::fs::read_to_string(&self.path).unwrap_or_default()
        }
    }

    /// AXL for a post-task hook that appends `post:<exit_code>:<message>`.
    fn post_hook(path: &str) -> String {
        format!(
            r#"
def _post(ctx, outcome):
    ctx.std.fs.try_append("{path}", "post:" + str(outcome.exit_code) + ":" + (outcome.message or "") + "\n")
"#
        )
    }

    /// A task whose body appends `body` then runs the AXL statement `tail`,
    /// with a feature registering a pre-task hook that appends `pre` and the
    /// [`post_hook`].
    fn bracketed_task(path: &str, tail: &str) -> String {
        format!(
            r#"
def _pre(ctx):
    ctx.std.fs.try_append("{path}", "pre\n")
{post}
def _feature(ctx):
    ctx.hooks.pre_task(_pre)
    ctx.hooks.post_task(_post)

def _impl(ctx):
    ctx.defer(ctx.std.fs.try_append, "{path}", "defer\n")
    ctx.std.fs.try_append("{path}", "body\n")
    {tail}

Hooks = feature(implementation = _feature)
t = task(implementation = _impl)
"#,
            post = post_hook(path),
        )
    }

    fn run_bracketed(tail: &str) -> (anyhow::Result<Option<u8>>, String) {
        let trace = Trace::new();
        let result = crate::test::eval(&bracketed_task(&trace.path, tail))
            .with_features(&["Hooks"])
            .run_task(0);
        (result, trace.read())
    }

    #[test]
    fn hooks_bracket_the_body_and_run_before_defers() {
        let (result, trace) = run_bracketed("return 0");
        assert_eq!(result.expect("run_task"), Some(0));
        assert_eq!(trace, "pre\nbody\npost:0:\ndefer\n");
    }

    #[test]
    fn a_returned_conclusion_reaches_the_post_hook() {
        let (result, trace) =
            run_bracketed(r#"return TaskConclusion(exit_code = 2, message = "nope")"#);
        assert_eq!(result.expect("run_task"), Some(2));
        assert_eq!(trace, "pre\nbody\npost:2:nope\ndefer\n");
    }

    #[test]
    fn an_exit_reaches_the_post_hook() {
        let (result, trace) = run_bracketed(r#"ctx.std.process.exit(3, "boom")"#);
        assert_eq!(result.expect("run_task"), Some(3));
        assert_eq!(trace, "pre\nbody\npost:3:boom\ndefer\n");
    }

    #[test]
    fn a_hard_error_reaches_the_post_hook_and_still_propagates() {
        let (result, trace) = run_bracketed(r#"fail("kaboom")"#);
        let err = result.expect_err("fail() must propagate");
        assert!(err.to_string().contains("kaboom"), "{err}");
        assert_eq!(trace, "pre\nbody\npost:1:fail: kaboom\ndefer\n");
    }

    #[test]
    fn a_pre_hook_exit_skips_the_body() {
        let trace = Trace::new();
        let exit = crate::test::eval(&format!(
            r#"
def _refuse(ctx):
    ctx.std.process.exit(4, "not today")
{post}
def _feature(ctx):
    ctx.hooks.pre_task(_refuse)
    ctx.hooks.post_task(_post)

def _impl(ctx):
    ctx.std.fs.try_append("{path}", "body\n")
    return 0

Hooks = feature(implementation = _feature)
t = task(implementation = _impl)
"#,
            path = trace.path,
            post = post_hook(&trace.path),
        ))
        .with_features(&["Hooks"])
        .run_task(0)
        .expect("run_task");
        assert_eq!(exit, Some(4));
        assert_eq!(trace.read(), "post:4:not today\n");
    }

    #[test]
    fn a_post_hook_registered_from_the_body_runs() {
        let trace = Trace::new();
        let exit = crate::test::eval(&format!(
            r#"
{post}
def _impl(ctx):
    ctx.hooks.post_task(_post)
    return 5

t = task(implementation = _impl)
"#,
            post = post_hook(&trace.path),
        ))
        .run_task(0)
        .expect("run_task");
        assert_eq!(exit, Some(5));
        assert_eq!(trace.read(), "post:5:\n");
    }

    #[test]
    fn a_pre_hook_cannot_be_registered_once_the_body_runs() {
        let err = crate::test::eval(
            r#"
def _impl(ctx):
    ctx.hooks.pre_task(lambda c: None)
    return 0

t = task(implementation = _impl)
"#,
        )
        .run_task(0)
        .expect_err("registering a pre-task hook from the body is an error");
        assert!(err.to_string().contains("already started"), "{err}");
    }

    #[test]
    fn a_failing_post_hook_changes_nothing_and_the_rest_still_run() {
        let trace = Trace::new();
        let exit = crate::test::eval(&format!(
            r#"
def _bad(ctx, outcome):
    fail("hook kaboom")

def _good(ctx, outcome):
    ctx.std.fs.try_append("{path}", "good\n")

def _impl(ctx):
    ctx.hooks.post_task(_bad)
    ctx.hooks.post_task(_good)
    return 0

t = task(implementation = _impl)
"#,
            path = trace.path,
        ))
        .run_task(0)
        .expect("run_task");
        assert_eq!(exit, Some(0));
        assert_eq!(trace.read(), "good\n");
    }

    /// `config.axl` evaluates before features, so its hooks come first in
    /// each list.
    #[test]
    fn config_hooks_run_before_feature_hooks() {
        let trace = Trace::new();
        let exit = crate::test::eval(&format!(
            r#"
def _feature(ctx):
    ctx.hooks.pre_task(lambda c: c.std.fs.try_append("{path}", "feature-pre\n"))
    ctx.hooks.post_task(lambda c, o: c.std.fs.try_append("{path}", "feature-post\n"))

def _impl(ctx):
    ctx.std.fs.try_append("{path}", "body\n")
    return 0

Hooks = feature(implementation = _feature)
t = task(implementation = _impl)
"#,
            path = trace.path,
        ))
        .with_config(&format!(
            r#"
def config(ctx):
    ctx.hooks.pre_task(lambda c: c.std.fs.try_append("{path}", "config-pre\n"))
    ctx.hooks.post_task(lambda c, o: c.std.fs.try_append("{path}", "config-post\n"))
"#,
            path = trace.path,
        ))
        .with_features(&["Hooks"])
        .run_task(0)
        .expect("run_task");
        assert_eq!(exit, Some(0));
        assert_eq!(
            trace.read(),
            "config-pre\nfeature-pre\nbody\nconfig-post\nfeature-post\n"
        );
    }

    /// The conclusion a post-task hook sees is final: a task that returned 0
    /// but left routed passthrough flags unclaimed has already been failed
    /// over them by the time the hook runs.
    #[test]
    fn post_hooks_see_the_exit_code_after_the_unclaimed_flag_check() {
        let trace = Trace::new();
        let exit = crate::test::eval(&format!(
            r#"
{post}
def _impl(ctx):
    ctx.hooks.post_task(_post)
    _ = ctx.args.rest
    return 0

t = task(implementation = _impl, args = {{"rest": args.passthrough(position = "post_command")}})
"#,
            post = post_hook(&trace.path),
        ))
        .with_string_list_args([("rest", vec!["--jobs=8"])])
        .run_task(0)
        .expect("run_task");
        assert_eq!(exit, Some(passthrough::EXIT_UNCLAIMED));
        assert_eq!(
            trace.read(),
            format!("post:{}:\n", passthrough::EXIT_UNCLAIMED)
        );
    }
}
