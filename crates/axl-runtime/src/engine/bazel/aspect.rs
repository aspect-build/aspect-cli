use std::sync::Arc;

use allocative::Allocative;
use bazel::server::Server;
use derive_more::Display;
use starlark::values;
use starlark::values::NoSerialize;
use starlark::values::ProvidesStaticType;
use starlark::values::Trace;
use starlark::values::UnpackValue;
use starlark::values::ValueLike;
use starlark::values::starlark_value;

use super::runner::SignalRunner;
use crate::engine::cancellation::Signals;

/// A Bazel aspect the runtime applies to a `build`/`test` invocation.
///
/// Created by `ctx.bazel.aspect(...)` and passed to
/// `ctx.bazel.build(aspects = [...])`. The runtime writes the aspect's source
/// into `@bazel_tools` — which is already in the repo graph, so applying the
/// aspect needs no `--inject_repository`/`--override_repository` and causes no
/// analysis-cache churn (the build reuses its warm analysis and just layers the
/// aspect nodes on top). See [`materialize`].
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

starlark::starlark_simple_value!(Aspect);

/// Materialize `aspects` into `@bazel_tools` and return the Bazel flags to
/// append to the invocation; see [`bazel::aspect::materialize`].
pub(crate) fn materialize(
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
