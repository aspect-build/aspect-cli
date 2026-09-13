//! The error a task raises to end early with a message and no traceback.
//!
//! Two producers: AXL calls `ctx.std.process.exit(code, message)`, and a Rust
//! builtin refusing a request returns [`TaskExit::error`] as its
//! `anyhow::Error`. Either unwinds through the evaluator like any other error,
//! so `ctx.defer` callbacks still run, and arrives wrapped in a
//! `starlark::Error`. Two consumers downcast it: the task runner in
//! `MultiPhaseEval::execute_tasks_with_args`, which reports it exactly like a
//! returned `TaskConclusion` with the same code and message, and the CLI's
//! top-level error arm, for an exit raised from a feature or config impl.
//! Neither prints a traceback unless `ASPECT_DEBUG` is set.
//!
//! The downcast finds `TaskExit` only at the root of the anyhow chain; a
//! `.context(...)` wrapper hides it and the error renders as a traceback.
//! The code is `NonZeroU8` on purpose: an early *success* would skip the hooks
//! the task body had yet to invoke while reporting a clean run, so exiting is
//! for refusals and `return 0` at the top of `_impl` remains the way to succeed
//! early.

use std::fmt;
use std::num::NonZeroU8;

use crate::diag;
use crate::errln;
use crate::eval::EvalError;

/// A task ending early: the exit code it wants and, optionally, why.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TaskExit {
    pub code: NonZeroU8,
    pub message: Option<String>,
}

impl TaskExit {
    pub fn new(code: NonZeroU8, message: Option<String>) -> Self {
        Self { code, message }
    }

    /// A refusal with exit code 1.
    pub fn error(message: impl Into<String>) -> Self {
        Self::new(NonZeroU8::MIN, Some(message.into()))
    }

    /// The exit carried by `err`, if that is what it is. `Native` is how a
    /// builtin's error arrives; `Other` is how [`EvalError`] re-wraps one.
    pub fn from_starlark(err: &starlark::Error) -> Option<&TaskExit> {
        match err.kind() {
            starlark::ErrorKind::Native(e) | starlark::ErrorKind::Other(e) => {
                e.downcast_ref::<TaskExit>()
            }
            _ => None,
        }
    }

    pub fn from_eval_error(err: &EvalError) -> Option<&TaskExit> {
        match err {
            EvalError::StarlarkError(e) => Self::from_starlark(e),
            EvalError::UnknownError(e) => Self::from_anyhow(e),
            _ => None,
        }
    }

    /// Search an `anyhow` chain for an exit, following [`EvalError`] links
    /// however deeply the phases have wrapped one another.
    pub fn from_anyhow(err: &anyhow::Error) -> Option<&TaskExit> {
        if let Some(exit) = err.downcast_ref::<TaskExit>() {
            return Some(exit);
        }
        err.downcast_ref::<EvalError>()
            .and_then(Self::from_eval_error)
    }

    /// Print the message as a red `ERROR:` line, matching the AXL `error()`
    /// helper a task would have used itself. `full` is the error as the
    /// evaluator raised it; under `ASPECT_DEBUG` it follows the message so the
    /// traceback is still available to whoever wants it.
    pub fn report(&self, full: &dyn fmt::Display) {
        if let Some(message) = &self.message {
            diag::error(message);
        }
        Self::debug_traceback(full);
    }

    /// Print the evaluator's full rendering of the error, traceback
    /// included, when `ASPECT_DEBUG` is set. Nothing otherwise.
    pub fn debug_traceback(full: &dyn fmt::Display) {
        if std::env::var_os("ASPECT_DEBUG").is_some_and(|v| !v.is_empty()) {
            errln!("{full}");
        }
    }
}

impl fmt::Display for TaskExit {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match &self.message {
            Some(message) => f.write_str(message),
            None => write!(f, "exit {}", self.code),
        }
    }
}

impl std::error::Error for TaskExit {}

#[cfg(test)]
mod tests {
    use super::*;

    fn run(body: &str) -> anyhow::Result<Option<u8>> {
        crate::test::eval(&format!(
            r#"
def _refuse(ctx):
    {body}

def _impl(ctx):
    _refuse(ctx)
    return 0

t = task(implementation = _impl)
"#
        ))
        .run_task(0)
    }

    #[test]
    fn nested_exit_becomes_the_exit_code() {
        let exit = run(r#"ctx.std.process.exit(3, "boom")"#).expect("run_task");
        assert_eq!(exit, Some(3));
    }

    #[test]
    fn exit_defaults_to_code_one_without_a_message() {
        let exit = run("ctx.std.process.exit()").expect("run_task");
        assert_eq!(exit, Some(1));
    }

    #[test]
    fn exit_rejects_zero_and_out_of_range_codes() {
        for code in ["0", "256", "-1"] {
            let err = run(&format!("ctx.std.process.exit({code})")).expect_err(code);
            let msg = err.to_string();
            assert!(msg.contains("exit code must be 1..=255"), "{code}: {msg}");
            assert!(
                msg.contains("Traceback"),
                "{code}: a bad code is a bug, so it keeps its trace: {msg}"
            );
        }
    }

    #[test]
    fn deferred_callbacks_still_run() {
        let dir = tempfile::tempdir().expect("temp dir");
        let marker = dir.path().join("deferred");
        let exit = run(&format!(
            r#"ctx.defer(lambda: ctx.std.fs.write("{}", "ran"))
    ctx.std.process.exit(2, "stop")"#,
            marker.display()
        ))
        .expect("run_task");
        assert_eq!(exit, Some(2));
        assert!(
            marker.exists(),
            "ctx.defer did not run before the exit was reported"
        );
    }

    #[test]
    fn native_task_exit_downcasts_but_plain_errors_do_not() {
        let exit = starlark::Error::new_native(anyhow::Error::new(TaskExit::error("x")));
        assert_eq!(TaskExit::from_starlark(&exit), Some(&TaskExit::error("x")));
        let plain = starlark::Error::new_native(anyhow::anyhow!("x"));
        assert_eq!(TaskExit::from_starlark(&plain), None);

        let wrapped: anyhow::Error = EvalError::from(exit).into();
        assert_eq!(
            TaskExit::from_anyhow(&wrapped).map(|e| e.code.get()),
            Some(1)
        );
        let context = wrapped.context("outer");
        assert_eq!(
            TaskExit::from_anyhow(&context).map(|e| e.code.get()),
            Some(1)
        );
    }

    /// A feature impl runs before the task runner is involved, so its exit
    /// reaches the caller as the eval error the CLI's top-level arm inspects.
    #[test]
    fn exit_from_a_feature_impl_is_found_in_the_eval_error() {
        let err = crate::test::eval(
            r#"
def _feature(ctx):
    ctx.std.process.exit(4, "not here")

def _impl(ctx):
    return 0

Guard = feature(implementation = _feature)
t = task(implementation = _impl)
"#,
        )
        .with_features(&["Guard"])
        .run_task(0)
        .expect_err("a feature impl exit is an error to the task runner");
        assert_eq!(TaskExit::from_anyhow(&err).map(|e| e.code.get()), Some(4));
    }

    #[test]
    fn display_is_the_message_or_the_code() {
        assert_eq!(TaskExit::error("nope").to_string(), "nope");
        assert_eq!(
            TaskExit::new(NonZeroU8::new(7).unwrap(), None).to_string(),
            "exit 7"
        );
    }
}
