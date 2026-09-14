//! How a task ended, resolved to one shape whichever way `_impl` finished.
//!
//! `_impl` can return a bare `int`, return a `TaskConclusion`, or raise a
//! [`TaskExit`]. Everything the runtime does afterwards, from printing the
//! message to choosing the bookend verdict, reads an [`Outcome`], so the three
//! endings cannot drift apart in how they render.

use starlark::values::Value;
use starlark::values::ValueLike;

use crate::diag;
use crate::engine::task_info::TaskConclusion;
use crate::eval::TaskExit;

#[derive(Debug, PartialEq, Eq)]
pub(crate) struct Outcome {
    /// `None` when `_impl` returned neither an `int` nor a `TaskConclusion`;
    /// the runtime renders that as a pass.
    pub(crate) exit_code: Option<u8>,
    pub(crate) flagged: bool,
    /// Bookend suffix, rendered as `· <text>` when non-empty.
    pub(crate) text: String,
    /// Why the task ended, printed on its own line before the bookend.
    pub(crate) message: Option<String>,
}

impl Outcome {
    /// The outcome of `_impl` returning `ret`: a `TaskConclusion` verbatim, a
    /// bare `int` as the exit code, anything else a pass with nothing to say.
    pub(crate) fn from_return(ret: Value<'_>) -> Self {
        if let Some(tc) = ret.downcast_ref::<TaskConclusion>() {
            Self {
                exit_code: Some(tc.exit_code as u8),
                flagged: tc.flagged,
                text: tc.text.clone(),
                message: tc.message.clone(),
            }
        } else {
            Self {
                exit_code: ret.unpack_i32().map(|code| code as u8),
                flagged: false,
                text: String::new(),
                message: None,
            }
        }
    }

    pub(crate) fn from_exit(exit: &TaskExit) -> Self {
        Self {
            exit_code: Some(exit.code),
            flagged: false,
            text: String::new(),
            message: exit.message.clone(),
        }
    }

    /// The severity `message` is printed with, following the verdict the
    /// bookend will show: `ERROR` for a failure, `WARNING` when flagged,
    /// `INFO` for a clean pass.
    pub(crate) fn severity(&self) -> diag::Severity {
        match self.exit_code {
            Some(code) if code != 0 => diag::Severity::Error,
            _ if self.flagged => diag::Severity::Warning,
            _ => diag::Severity::Info,
        }
    }

    /// Print `message`, if any, at [`Self::severity`].
    pub(crate) fn report_message(&self) {
        if let Some(message) = &self.message {
            diag::emit(self.severity(), message);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::diag::Severity;

    fn outcome(exit_code: Option<u8>, flagged: bool) -> Outcome {
        Outcome {
            exit_code,
            flagged,
            text: String::new(),
            message: Some("why".to_string()),
        }
    }

    fn from_return_of(expr: &str) -> Outcome {
        crate::test::eval(&format!("x = {expr}")).with_value("x", Outcome::from_return)
    }

    #[test]
    fn message_severity_follows_the_verdict() {
        assert_eq!(outcome(Some(1), false).severity(), Severity::Error);
        // Failure dominates the flag, as it does on the bookend.
        assert_eq!(outcome(Some(1), true).severity(), Severity::Error);
        assert_eq!(outcome(Some(0), true).severity(), Severity::Warning);
        assert_eq!(outcome(Some(0), false).severity(), Severity::Info);
        assert_eq!(outcome(None, false).severity(), Severity::Info);
    }

    #[test]
    fn a_returned_conclusion_is_taken_verbatim() {
        assert_eq!(
            from_return_of(
                r#"TaskConclusion(exit_code = 2, text = "t", flagged = True, message = "m")"#
            ),
            Outcome {
                exit_code: Some(2),
                flagged: true,
                text: "t".to_string(),
                message: Some("m".to_string()),
            }
        );
    }

    #[test]
    fn a_returned_int_is_the_exit_code() {
        assert_eq!(
            from_return_of("7"),
            Outcome {
                exit_code: Some(7),
                flagged: false,
                text: String::new(),
                message: None,
            }
        );
    }

    #[test]
    fn any_other_return_is_a_pass_with_nothing_to_say() {
        assert_eq!(from_return_of("None").exit_code, None);
        assert_eq!(from_return_of(r#""done""#).exit_code, None);
    }

    #[test]
    fn an_exit_carries_its_code_and_message() {
        assert_eq!(
            Outcome::from_exit(&TaskExit::new(3, Some("stop".to_string()))),
            Outcome {
                exit_code: Some(3),
                flagged: false,
                text: String::new(),
                message: Some("stop".to_string()),
            }
        );
    }
}
