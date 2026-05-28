use std::fmt;

use thiserror::Error;

/// Enum representing possible errors during evaluation, including Starlark-specific errors,
/// missing symbols, and wrapped anyhow or IO errors.
#[derive(Error, Debug)]
#[non_exhaustive]
pub enum EvalError {
    #[error("{}", StarlarkErrorDisplay(.0))]
    StarlarkError(starlark::Error),

    #[error("{0:?}")]
    FreezeError(starlark::values::FreezeError),

    #[error("script does not export {0:?} symbol")]
    MissingSymbol(String),

    #[error(transparent)]
    UnknownError(#[from] anyhow::Error),

    #[error(transparent)]
    IOError(#[from] std::io::Error),
}

impl EvalError {
    /// True when this error is a pre-shaped user-facing block that should be
    /// rendered without the AXL call-stack traceback. Currently any
    /// [`bazelrc::BazelRcError`] qualifies — those messages already name the
    /// subsystem, the source of the bad flag, the loaded rc files, and the
    /// fix; the call stack only adds noise.
    ///
    /// The `Display` impl uses this same check to strip the traceback inline.
    /// The CLI's main loop calls this from outside to decide between Display
    /// (`{err}`, no traceback) and Debug (`{err:?}`, full diagnostic).
    pub fn is_pre_shaped_user_error(&self) -> bool {
        match self {
            EvalError::StarlarkError(s) => starlark_wraps_user_error(s),
            _ => false,
        }
    }
}

/// True when `s` carries an [`anyhow::Error`] that downcasts to a
/// [`bazelrc::BazelRcError`]. Only `ErrorKind::Native` and `ErrorKind::Other`
/// carry an inner `anyhow::Error` we can downcast; everything else is internal
/// to starlark and never wraps our errors.
fn starlark_wraps_user_error(s: &starlark::Error) -> bool {
    let anyhow_err = match s.kind() {
        starlark::ErrorKind::Native(e) | starlark::ErrorKind::Other(e) => e,
        _ => return false,
    };
    anyhow_err.downcast_ref::<bazelrc::BazelRcError>().is_some()
}

/// `Display` for [`EvalError::StarlarkError`] that strips the AXL call-stack
/// traceback when the error wraps a pre-shaped user-facing block. See
/// [`EvalError::is_pre_shaped_user_error`].
struct StarlarkErrorDisplay<'a>(&'a starlark::Error);

impl<'a> fmt::Display for StarlarkErrorDisplay<'a> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if starlark_wraps_user_error(self.0) {
            write!(f, "{}", self.0.without_diagnostic())
        } else {
            write!(f, "{}", self.0)
        }
    }
}

// Custom From implementation since starlark::Error doesn't implement std::error::Error.
impl From<starlark::Error> for EvalError {
    fn from(value: starlark::Error) -> Self {
        Self::StarlarkError(value)
    }
}

impl Into<starlark::Error> for EvalError {
    fn into(self) -> starlark::Error {
        match self {
            EvalError::StarlarkError(error) => error,
            EvalError::MissingSymbol(_) => starlark::Error::new_other(self),
            EvalError::UnknownError(error) => starlark::Error::new_other(error),
            EvalError::IOError(error) => starlark::Error::new_other(error),
            EvalError::FreezeError(error) => starlark::Error::new_other(error),
        }
    }
}
