use thiserror::Error;

/// Enum representing possible errors during evaluation, including Starlark-specific errors,
/// missing symbols, and wrapped anyhow or IO errors.
#[derive(Error, Debug)]
#[non_exhaustive]
pub enum EvalError {
    #[error("{0}")]
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
