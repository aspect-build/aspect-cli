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
    AllocValue, Freeze, FreezeError, Freezer, FrozenValue, Heap, NoSerialize, ProvidesStaticType,
    StarlarkValue, Trace, Tracer, Value, ValueLike, starlark_value,
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

    /// Mark the body as about to run; pre-task registration is refused from
    /// here on.
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
    fn freeze(self, freezer: &Freezer) -> Result<Self::Frozen, FreezeError> {
        let freeze_all = |hooks: Vec<Value<'v>>| -> Result<Vec<FrozenValue>, FreezeError> {
            hooks.into_iter().map(|h| h.freeze(freezer)).collect()
        };
        Ok(FrozenTaskHooks {
            pre: freeze_all(self.pre.into_inner())?,
            post: freeze_all(self.post.into_inner())?,
        })
    }
}

#[starlark_value(type = "TaskHooks")]
impl<'v> StarlarkValue<'v> for TaskHooks<'v> {
    fn get_methods() -> Option<&'static Methods> {
        static RES: MethodsStatic = MethodsStatic::new();
        RES.methods(task_hooks_methods)
    }
}

/// The frozen view keeps the registered hooks for inspection but accepts no
/// more: a frozen module has no run to register into.
#[derive(Debug, ProvidesStaticType, NoSerialize, Allocative, Display)]
#[display("<TaskHooks>")]
pub struct FrozenTaskHooks {
    #[allocative(skip)]
    pre: Vec<FrozenValue>,
    #[allocative(skip)]
    post: Vec<FrozenValue>,
}

starlark::starlark_simple_value!(FrozenTaskHooks);

#[starlark_value(type = "TaskHooks")]
impl<'v> StarlarkValue<'v> for FrozenTaskHooks {
    type Canonical = TaskHooks<'v>;

    fn get_methods() -> Option<&'static Methods> {
        static RES: MethodsStatic = MethodsStatic::new();
        RES.methods(task_hooks_methods)
    }
}

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
                "pre_task: the task body has already started; use post_task or ctx.defer"
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
    /// A task whose body appends `body` to `path`, with a feature registering
    /// a pre-task hook that appends `pre` and a post-task hook that appends
    /// `post:<exit_code>:<message>`. `body` is the AXL statement the body runs
    /// after its append.
    fn script(path: &str, body: &str) -> String {
        format!(
            r#"
def _pre(ctx):
    ctx.std.fs.try_append("{path}", "pre\n")

def _post(ctx, outcome):
    ctx.std.fs.try_append("{path}", "post:" + str(outcome.exit_code) + ":" + (outcome.message or "") + "\n")

def _feature(ctx):
    ctx.hooks.pre_task(_pre)
    ctx.hooks.post_task(_post)

def _impl(ctx):
    ctx.defer(ctx.std.fs.try_append, "{path}", "defer\n")
    ctx.std.fs.try_append("{path}", "body\n")
    {body}

Hooks = feature(implementation = _feature)
t = task(implementation = _impl)
"#
        )
    }

    fn run(body: &str) -> (anyhow::Result<Option<u8>>, String) {
        let dir = tempfile::tempdir().expect("temp dir");
        let trace = dir.path().join("trace");
        let result = crate::test::eval(&script(&trace.to_string_lossy(), body))
            .with_features(&["Hooks"])
            .run_task(0);
        let contents = std::fs::read_to_string(&trace).unwrap_or_default();
        (result, contents)
    }

    #[test]
    fn hooks_bracket_the_body_and_run_before_defers() {
        let (result, trace) = run("return 0");
        assert_eq!(result.expect("run_task"), Some(0));
        assert_eq!(trace, "pre\nbody\npost:0:\ndefer\n");
    }

    #[test]
    fn a_returned_conclusion_reaches_the_post_hook() {
        let (result, trace) = run(r#"return TaskConclusion(exit_code = 2, message = "nope")"#);
        assert_eq!(result.expect("run_task"), Some(2));
        assert_eq!(trace, "pre\nbody\npost:2:nope\ndefer\n");
    }

    #[test]
    fn an_exit_reaches_the_post_hook() {
        let (result, trace) = run(r#"ctx.std.process.exit(3, "boom")"#);
        assert_eq!(result.expect("run_task"), Some(3));
        assert_eq!(trace, "pre\nbody\npost:3:boom\ndefer\n");
    }

    #[test]
    fn a_hard_error_reaches_the_post_hook_and_still_propagates() {
        let (result, trace) = run(r#"fail("kaboom")"#);
        let err = result.expect_err("fail() must propagate");
        assert!(err.to_string().contains("kaboom"), "{err}");
        assert_eq!(trace, "pre\nbody\npost:1:fail: kaboom\ndefer\n");
    }

    #[test]
    fn a_pre_hook_exit_skips_the_body() {
        let dir = tempfile::tempdir().expect("temp dir");
        let trace = dir.path().join("trace");
        let path = trace.to_string_lossy();
        let exit = crate::test::eval(&format!(
            r#"
def _refuse(ctx):
    ctx.std.process.exit(4, "not today")

def _post(ctx, outcome):
    ctx.std.fs.try_append("{path}", "post:" + str(outcome.exit_code) + ":" + outcome.message + "\n")

def _feature(ctx):
    ctx.hooks.pre_task(_refuse)
    ctx.hooks.post_task(_post)

def _impl(ctx):
    ctx.std.fs.try_append("{path}", "body\n")
    return 0

Hooks = feature(implementation = _feature)
t = task(implementation = _impl)
"#
        ))
        .with_features(&["Hooks"])
        .run_task(0)
        .expect("run_task");
        assert_eq!(exit, Some(4));
        assert_eq!(
            std::fs::read_to_string(&trace).unwrap(),
            "post:4:not today\n"
        );
    }

    #[test]
    fn a_post_hook_registered_from_the_body_runs() {
        let dir = tempfile::tempdir().expect("temp dir");
        let trace = dir.path().join("trace");
        let path = trace.to_string_lossy();
        let exit = crate::test::eval(&format!(
            r#"
def _impl(ctx):
    ctx.hooks.post_task(lambda c, outcome: c.std.fs.try_append("{path}", "post:" + str(outcome.exit_code) + "\n"))
    return 5

t = task(implementation = _impl)
"#
        ))
        .run_task(0)
        .expect("run_task");
        assert_eq!(exit, Some(5));
        assert_eq!(std::fs::read_to_string(&trace).unwrap(), "post:5\n");
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
        let dir = tempfile::tempdir().expect("temp dir");
        let trace = dir.path().join("trace");
        let path = trace.to_string_lossy();
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
"#
        ))
        .run_task(0)
        .expect("run_task");
        assert_eq!(exit, Some(0));
        assert_eq!(std::fs::read_to_string(&trace).unwrap(), "good\n");
    }

    #[test]
    fn hooks_registered_from_config_axl_run() {
        let dir = tempfile::tempdir().expect("temp dir");
        let trace = dir.path().join("trace");
        let path = trace.to_string_lossy();
        let exit = crate::test::eval(&format!(
            r#"
def _impl(ctx):
    ctx.std.fs.try_append("{path}", "body\n")
    return 0

t = task(implementation = _impl)
"#
        ))
        .with_config(&format!(
            r#"
def _pre(ctx):
    ctx.std.fs.try_append("{path}", "pre\n")

def _post(ctx, outcome):
    ctx.std.fs.try_append("{path}", "post:" + str(outcome.exit_code) + "\n")

def config(ctx):
    ctx.hooks.pre_task(_pre)
    ctx.hooks.post_task(_post)
"#
        ))
        .run_task(0)
        .expect("run_task");
        assert_eq!(exit, Some(0));
        assert_eq!(
            std::fs::read_to_string(&trace).unwrap(),
            "pre\nbody\npost:0\n"
        );
    }
}
