use allocative::Allocative;
use derive_more::Display;

use starlark::environment::{Methods, MethodsBuilder, MethodsStatic};
use starlark::starlark_module;
use starlark::values::{
    AllocValue, Freeze, FreezeError, Freezer, FrozenValue, Heap, NoSerialize, ProvidesStaticType,
    StarlarkValue, Trace, Tracer, Value, ValueLike, starlark_value,
};

use super::aspect::Aspect;
use super::http::Http;
use super::std::Std;

/// Context passed to a feature's `implementation` function.
///
/// Runs after all config.axl files have been evaluated.
/// - `ctx.args` — the feature's resolved args (config values and CLI args merged)
/// - `ctx.traits` — the full mutable fragment map (inject hooks into fragments)
/// - `ctx.std` — standard library
/// - `ctx.http` — HTTP client
#[derive(Debug, ProvidesStaticType, NoSerialize, Allocative, Display)]
#[display("<FeatureContext>")]
pub struct FeatureContext<'v> {
    /// Resolved args: config-only args from the feature instance, plus CLI args.
    pub(crate) args: Value<'v>,
    /// The full mutable fragment map — inject hooks here.
    pub(crate) fragments: Value<'v>,
    /// Telemetry handle: `ctx.telemetry.exporters.add(...)` registers OTLP
    /// exporters that the runtime installs after phase 3 completes.
    pub(crate) telemetry: Value<'v>,
}

unsafe impl<'v> Trace<'v> for FeatureContext<'v> {
    fn trace(&mut self, tracer: &Tracer<'v>) {
        self.args.trace(tracer);
        self.fragments.trace(tracer);
        self.telemetry.trace(tracer);
    }
}

impl<'v> FeatureContext<'v> {
    pub fn new(args: Value<'v>, fragments: Value<'v>, telemetry: Value<'v>) -> Self {
        Self {
            args,
            fragments,
            telemetry,
        }
    }
}

impl<'v> AllocValue<'v> for FeatureContext<'v> {
    fn alloc_value(self, heap: Heap<'v>) -> Value<'v> {
        heap.alloc_complex(self)
    }
}

impl<'v> Freeze for FeatureContext<'v> {
    type Frozen = FrozenFeatureContext;

    fn freeze(self, freezer: &Freezer) -> Result<Self::Frozen, FreezeError> {
        Ok(FrozenFeatureContext {
            args: self.args.freeze(freezer)?,
            fragments: self.fragments.freeze(freezer)?,
            telemetry: self.telemetry.freeze(freezer)?,
        })
    }
}

#[starlark_value(type = "FeatureContext")]
impl<'v> StarlarkValue<'v> for FeatureContext<'v> {
    fn get_methods() -> Option<&'static Methods> {
        static RES: MethodsStatic = MethodsStatic::new();
        RES.methods(feature_context_methods)
    }
}

#[derive(Debug, ProvidesStaticType, NoSerialize, Allocative, Display)]
#[display("<FeatureContext>")]
pub struct FrozenFeatureContext {
    #[allocative(skip)]
    args: FrozenValue,
    #[allocative(skip)]
    fragments: FrozenValue,
    #[allocative(skip)]
    telemetry: FrozenValue,
}

unsafe impl<'v> Trace<'v> for FrozenFeatureContext {
    fn trace(&mut self, _tracer: &Tracer<'v>) {}
}

#[starlark_value(type = "FeatureContext")]
impl<'v> StarlarkValue<'v> for FrozenFeatureContext {
    type Canonical = FeatureContext<'v>;

    fn get_methods() -> Option<&'static Methods> {
        static RES: MethodsStatic = MethodsStatic::new();
        RES.methods(feature_context_methods)
    }
}

#[starlark_module]
fn feature_context_methods(builder: &mut MethodsBuilder) {
    /// Resolved args for this feature: config-only values and CLI args merged.
    /// Access via `ctx.args.arg_name`.
    #[starlark(attribute)]
    fn args<'v>(this: Value<'v>) -> anyhow::Result<Value<'v>> {
        let ctx = this
            .downcast_ref_err::<FeatureContext>()
            .map_err(|e| anyhow::anyhow!("{}", e))?;
        Ok(ctx.args)
    }

    /// The full mutable trait map. Inject into traits via `ctx.traits[TraitType].hook.append(...)`.
    #[starlark(attribute)]
    fn traits<'v>(this: Value<'v>) -> anyhow::Result<Value<'v>> {
        let ctx = this
            .downcast_ref_err::<FeatureContext>()
            .map_err(|e| anyhow::anyhow!("{}", e))?;
        Ok(ctx.fragments)
    }

    /// Standard library — same as `ctx.std` in config and task functions.
    #[starlark(attribute)]
    fn std<'v>(#[allow(unused)] this: Value<'v>) -> anyhow::Result<Std> {
        Ok(Std {})
    }

    /// Aspect platform APIs (auth, etc.).
    #[starlark(attribute)]
    fn aspect<'v>(#[allow(unused)] this: Value<'v>) -> anyhow::Result<Aspect> {
        Ok(Aspect {})
    }

    /// HTTP client for making requests during feature initialization.
    fn http<'v>(#[allow(unused)] this: Value<'v>) -> anyhow::Result<Http> {
        Ok(Http::new())
    }

    /// Telemetry handle. Use `ctx.telemetry.exporters.add(url=..., ...)` to
    /// register OTLP exporters that the runtime installs after phase 3.
    /// Buffered spans/logs from earlier phases are replayed to them.
    #[starlark(attribute)]
    fn telemetry<'v>(this: Value<'v>) -> anyhow::Result<Value<'v>> {
        if let Some(c) = this.downcast_ref::<FeatureContext>() {
            return Ok(c.telemetry);
        }
        if let Some(c) = this.downcast_ref::<FrozenFeatureContext>() {
            return Ok(c.telemetry.to_value());
        }
        Err(anyhow::anyhow!("expected FeatureContext"))
    }
}
