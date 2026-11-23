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

    /// A config file with no `config` function to call. Named by path, since
    /// a repository's and a user's config can both be in play.
    #[error("{0} does not define `def config(ctx)`")]
    MissingConfig(std::path::PathBuf),

    /// Trait fields a config left holding a value of the wrong type — by
    /// mutating a list field in place, which assignment's own check never sees.
    #[error("after running config files, {}", .0.join("; "))]
    TraitTypeMismatch(Vec<String>),

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

/// The `.bazelrc` problem `err` carries, however the evaluator and the phases
/// wrapped it — what the CLI prints in place of a traceback.
pub fn rc_resolution_error(err: &anyhow::Error) -> Option<&bazel::rc::RcResolutionError> {
    err.chain().find_map(|e| {
        if let Some(found) = e.downcast_ref::<bazel::rc::RcResolutionError>() {
            return Some(found);
        }
        match e.downcast_ref::<EvalError>()? {
            EvalError::StarlarkError(e) => match e.kind() {
                starlark::ErrorKind::Native(e) | starlark::ErrorKind::Other(e) => {
                    rc_resolution_error(e)
                }
                _ => None,
            },
            EvalError::UnknownError(e) => rc_resolution_error(e),
            _ => None,
        }
    })
}

impl Into<starlark::Error> for EvalError {
    fn into(self) -> starlark::Error {
        match self {
            EvalError::StarlarkError(error) => error,
            EvalError::MissingSymbol(_) => starlark::Error::new_other(self),
            EvalError::MissingConfig(_) => starlark::Error::new_other(self),
            EvalError::TraitTypeMismatch(_) => starlark::Error::new_other(self),
            EvalError::UnknownError(error) => starlark::Error::new_other(error),
            EvalError::IOError(error) => starlark::Error::new_other(error),
            EvalError::FreezeError(error) => starlark::Error::new_other(error),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use bazel::rc::{BazelrcError, RcResolutionError};

    fn rc_error() -> anyhow::Error {
        RcResolutionError::from(BazelrcError::ConfigCycle {
            cycle: vec!["a".into(), "a".into()],
        })
        .into()
    }

    #[test]
    fn finds_an_rc_error_under_the_evaluator_and_its_context() {
        let native: anyhow::Error = EvalError::from(starlark::Error::new_native(rc_error())).into();
        let err = native.context("task failed");
        assert_eq!(
            rc_resolution_error(&err)
                .map(ToString::to_string)
                .as_deref(),
            Some("bazelrc: --config expansion cycle: a → a")
        );

        let unknown: anyhow::Error = EvalError::UnknownError(rc_error().context("setup")).into();
        assert!(rc_resolution_error(&unknown).is_some());
    }

    #[test]
    fn ignores_other_errors() {
        let err: anyhow::Error =
            EvalError::from(starlark::Error::new_native(anyhow::anyhow!("boom"))).into();
        assert!(rc_resolution_error(&err).is_none());
    }
}
