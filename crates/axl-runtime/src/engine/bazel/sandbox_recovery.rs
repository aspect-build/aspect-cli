//! The AXL-facing result of [`bazel::server::sandbox::recover`].

use allocative::Allocative;
use bazel::server::sandbox::RecoveryOutcome;
use derive_more::Display;
use starlark::environment::{Methods, MethodsBuilder, MethodsStatic};
use starlark::starlark_module;
use starlark::starlark_simple_value;
use starlark::values;
use starlark::values::starlark_value;
use starlark::values::{NoSerialize, ProvidesStaticType, ValueLike};

/// Starlark-facing wrapper around [`RecoveryOutcome`]. Returned by
/// `ctx.bazel.recover_poisoned_sandbox()`.
#[derive(Debug, Display, ProvidesStaticType, NoSerialize, Allocative)]
#[display("<bazel.SandboxRecoveryResult>")]
pub struct SandboxRecoveryResult {
    outcome: String,
    removed: Vec<String>,
    remaining: Vec<String>,
}

impl SandboxRecoveryResult {
    pub(crate) fn from_outcome(outcome: RecoveryOutcome) -> Self {
        Self {
            outcome: outcome.tag().to_string(),
            removed: outcome.removed_entries(),
            remaining: outcome.remaining_entries(),
        }
    }

    /// Builder used when the caller skipped recovery (e.g. no
    /// `--output_base` in startup flags). Reads as `clean` so callers
    /// can branch uniformly on the outcome tag.
    pub(crate) fn skipped() -> Self {
        Self {
            outcome: "clean".to_string(),
            removed: Vec::new(),
            remaining: Vec::new(),
        }
    }
}

starlark_simple_value!(SandboxRecoveryResult);

#[starlark_value(type = "bazel.SandboxRecoveryResult")]
impl<'v> values::StarlarkValue<'v> for SandboxRecoveryResult {
    fn get_methods() -> Option<&'static Methods> {
        static RES: MethodsStatic = MethodsStatic::new(
            "sandbox_recovery_result_methods",
            sandbox_recovery_result_methods,
        );
        Some(RES.methods())
    }
}

#[starlark_module]
pub(crate) fn sandbox_recovery_result_methods(registry: &mut MethodsBuilder) {
    /// Recovery outcome: `"clean"`, `"repaired"`, or `"still_poisoned"`.
    ///
    /// - `clean`: nothing to do. Either the sandbox dir didn't exist or
    ///   contained only whitelisted entries.
    /// - `repaired`: non-whitelisted entries were found and successfully
    ///   removed. The runner is usable for the next job.
    /// - `still_poisoned`: non-whitelisted entries were found and at least
    ///   one could not be removed. Caller should signal the runner
    ///   unhealthy.
    #[starlark(attribute)]
    fn outcome<'v>(this: values::Value<'v>) -> anyhow::Result<String> {
        Ok(this
            .downcast_ref::<SandboxRecoveryResult>()
            .unwrap()
            .outcome
            .clone())
    }

    /// Sorted names of sandbox-base entries that were successfully removed.
    /// Empty when `outcome == "clean"`.
    #[starlark(attribute)]
    fn removed<'v>(this: values::Value<'v>) -> anyhow::Result<Vec<String>> {
        Ok(this
            .downcast_ref::<SandboxRecoveryResult>()
            .unwrap()
            .removed
            .clone())
    }

    /// Sorted names of sandbox-base entries that survived the removal
    /// attempt. Only non-empty when `outcome == "still_poisoned"`.
    #[starlark(attribute)]
    fn remaining<'v>(this: values::Value<'v>) -> anyhow::Result<Vec<String>> {
        Ok(this
            .downcast_ref::<SandboxRecoveryResult>()
            .unwrap()
            .remaining
            .clone())
    }
}
