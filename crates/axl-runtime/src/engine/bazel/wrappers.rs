//! The Starlark values `ctx.bazel` takes and returns that wrap what the
//! [`bazel`] crate computes: a script's view of a health check, a sandbox
//! recovery, an aspect and an execution-log sink.

use std::sync::Arc;

use allocative::Allocative;
use bazel::server::Server;
use bazel::server::sandbox::RecoveryOutcome;
use derive_more::Display;
use starlark::environment::{Methods, MethodsBuilder, MethodsStatic};
use starlark::starlark_module;
use starlark::starlark_simple_value;
use starlark::values;
use starlark::values::none::NoneOr;
use starlark::values::starlark_value;
use starlark::values::{NoSerialize, ProvidesStaticType, Trace, UnpackValue, ValueLike};

use super::runner::SignalRunner;
use crate::engine::cancellation::Signals;

/// Probe the Bazel server and recover from a held lock or a wedged server
/// when possible. `log` receives one human-readable line per step taken, for
/// the job log.
pub fn health_check(
    signals: &Arc<Signals>,
    startup_flags: &[String],
    log: &mut dyn FnMut(&str),
) -> HealthCheckResult {
    Server::new(&SignalRunner(signals), startup_flags)
        .health(log)
        .into()
}

#[derive(Debug, Display, ProvidesStaticType, NoSerialize, Allocative)]
#[display("<bazel.HealthCheckResult>")]
pub struct HealthCheckResult {
    /// One of "healthy", "unhealthy", or "inconclusive".
    outcome: String,
    message: Option<String>,
    exit_code: Option<i32>,
}

impl From<bazel::server::health::HealthCheckResult> for HealthCheckResult {
    fn from(result: bazel::server::health::HealthCheckResult) -> Self {
        Self {
            outcome: result.outcome,
            message: result.message,
            exit_code: result.exit_code,
        }
    }
}

starlark_simple_value!(HealthCheckResult);

#[starlark_value(type = "bazel.HealthCheckResult")]
impl<'v> values::StarlarkValue<'v> for HealthCheckResult {
    fn get_methods() -> Option<&'static Methods> {
        static RES: MethodsStatic =
            MethodsStatic::new("health_check_result_methods", health_check_result_methods);
        Some(RES.methods())
    }
}

#[starlark_module]
fn health_check_result_methods(registry: &mut MethodsBuilder) {
    /// The server health state: `"healthy"`, `"unhealthy"`, or `"inconclusive"`.
    #[starlark(attribute)]
    fn outcome<'v>(this: values::Value<'v>) -> anyhow::Result<String> {
        Ok(this
            .downcast_ref::<HealthCheckResult>()
            .unwrap()
            .outcome
            .clone())
    }

    /// Diagnostic message, if any.
    #[starlark(attribute)]
    fn message<'v>(this: values::Value<'v>) -> anyhow::Result<NoneOr<String>> {
        Ok(NoneOr::from_option(
            this.downcast_ref::<HealthCheckResult>()
                .unwrap()
                .message
                .clone(),
        ))
    }

    /// The original Bazel exit code, if available.
    #[starlark(attribute)]
    fn exit_code<'v>(this: values::Value<'v>) -> anyhow::Result<NoneOr<i32>> {
        Ok(NoneOr::from_option(
            this.downcast_ref::<HealthCheckResult>().unwrap().exit_code,
        ))
    }
}

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
fn sandbox_recovery_result_methods(registry: &mut MethodsBuilder) {
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

/// A Bazel aspect the runtime applies to a `build`/`test` invocation.
///
/// Created by `ctx.bazel.aspect(...)` and passed to
/// `ctx.bazel.build(aspects = [...])`. The runtime writes the aspect's source
/// into `@bazel_tools` — which is already in the repo graph, so applying the
/// aspect needs no `--inject_repository`/`--override_repository` and causes no
/// analysis-cache churn (the build reuses its warm analysis and just layers the
/// aspect nodes on top). See [`materialize_aspects`].
#[derive(Clone, Debug, ProvidesStaticType, Display, Trace, NoSerialize, Allocative)]
#[display("<bazel.Aspect %{symbol}>")]
pub(crate) struct Aspect {
    /// The `.bzl` source defining the aspect.
    pub implementation: String,
    /// The aspect symbol within `implementation` (the `%name`).
    pub symbol: String,
    /// `--aspects_parameters=<k>=<v>` pairs. A value that varies per run (e.g. a
    /// nonce) mints a fresh `AspectKey`, forcing the aspect to re-evaluate.
    pub parameters: Vec<(String, String)>,
    /// Output groups to request so the aspect's outputs build
    /// (`--output_groups=+<g>`).
    pub output_groups: Vec<String>,
}

impl<'v> UnpackValue<'v> for Aspect {
    type Error = anyhow::Error;

    // `Ok(None)` on type mismatch so `UnpackList`/`Either` report a clean error.
    fn unpack_value_impl(value: values::Value<'v>) -> Result<Option<Self>, Self::Error> {
        Ok(value.downcast_ref::<Aspect>().cloned())
    }
}

#[starlark_value(type = "bazel.Aspect")]
impl<'v> values::StarlarkValue<'v> for Aspect {}

starlark_simple_value!(Aspect);

/// Materialize `aspects` into `@bazel_tools` and return the Bazel flags to
/// append to the invocation; see [`bazel::aspect::materialize`].
pub(crate) fn materialize_aspects(
    signals: &Arc<Signals>,
    aspects: &[Aspect],
    startup_flags: &[String],
) -> anyhow::Result<Vec<String>> {
    let aspects: Vec<bazel::aspect::Aspect> = aspects
        .iter()
        .map(|a| bazel::aspect::Aspect {
            implementation: a.implementation.clone(),
            symbol: a.symbol.clone(),
            parameters: a.parameters.clone(),
            output_groups: a.output_groups.clone(),
        })
        .collect();
    let runner = SignalRunner(signals);
    bazel::aspect::materialize(&Server::new(&runner, startup_flags), &aspects)
}

/// Where a build's execution log goes.
///
/// | Variant | Format |
/// |---|---|
/// | `File` | Varint-length-prefixed binary proto, no zstd (decoded entries re-encoded) |
/// | `CompactFile` | Raw zstd-compressed bytes (identical to `--execution_log_compact_file`) |
#[derive(Debug, Display, ProvidesStaticType, NoSerialize, Allocative, Clone)]
#[display("<bazel.execlog.ExecLogSink>")]
pub enum ExecLogSink {
    File { path: String },
    CompactFile { path: String },
}

starlark_simple_value!(ExecLogSink);

#[starlark_value(type = "bazel.execlog.ExecLogSink")]
impl<'v> values::StarlarkValue<'v> for ExecLogSink {}

impl<'v> UnpackValue<'v> for ExecLogSink {
    type Error = anyhow::Error;

    // `Ok(None)` (not `Err`) on type mismatch so Either's UnpackValue can fall
    // through to the next branch — `execution_log=` takes a list of
    // `ExecLogSink | ExecLogIter`, and an `Err` here would reject every iterator
    // handle before the iterator branch was ever tried. Same reasoning, and the
    // same shape, as `BuildEventSink`.
    fn unpack_value_impl(value: values::Value<'v>) -> Result<Option<Self>, Self::Error> {
        Ok(value.downcast_ref::<ExecLogSink>().cloned())
    }
}
