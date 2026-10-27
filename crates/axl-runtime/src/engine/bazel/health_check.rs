//! The AXL-facing result of [`bazel::server::health`].

use std::sync::Arc;

use allocative::Allocative;
use bazel::server::Server;
use derive_more::Display;
use starlark::environment::{Methods, MethodsBuilder, MethodsStatic};
use starlark::starlark_module;
use starlark::starlark_simple_value;
use starlark::values;
use starlark::values::none::NoneOr;
use starlark::values::starlark_value;
use starlark::values::{NoSerialize, ProvidesStaticType, ValueLike};

use super::runner::SignalRunner;
use crate::engine::cancellation::Signals;

/// Probe the Bazel server and recover from a held lock or a wedged server
/// when possible. `log` receives one human-readable line per step taken, for
/// the job log.
pub fn run(
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
pub(crate) fn health_check_result_methods(registry: &mut MethodsBuilder) {
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
