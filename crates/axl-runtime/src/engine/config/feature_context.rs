use allocative::Allocative;
use derive_more::Display;

use starlark::environment::{Methods, MethodsBuilder, MethodsStatic};
use starlark::starlark_module;
use starlark::values::{
    AllocValue, Freeze, FreezeError, Freezer, FrozenValue, Heap, NoSerialize, ProvidesStaticType,
    StarlarkValue, Trace, Tracer, Value, ValueLike, starlark_value,
};

use crate::engine::http::Http;
use crate::engine::std::Std;

/// Context passed to a feature's `implementation` function.
///
/// Runs after all config.axl files have been evaluated.
/// - `ctx.attr` — the feature's own configured instance (read its user-set fields)
/// - `ctx.fragments` — the full mutable fragment map (inject hooks into fragments)
/// - `ctx.std` — standard library
/// - `ctx.http` — HTTP client
#[derive(Debug, ProvidesStaticType, NoSerialize, Allocative, Display)]
#[display("<FeatureContext>")]
pub struct FeatureContext<'v> {
    /// The feature instance (read-only at implementation time — config phase is done).
    #[allocative(skip)]
    pub(crate) attr: Value<'v>,
    /// The full mutable fragment map — inject hooks here.
    #[allocative(skip)]
    pub(crate) fragments: Value<'v>,
}

unsafe impl<'v> Trace<'v> for FeatureContext<'v> {
    fn trace(&mut self, tracer: &Tracer<'v>) {
        self.attr.trace(tracer);
        self.fragments.trace(tracer);
    }
}

impl<'v> FeatureContext<'v> {
    pub fn new(attr: Value<'v>, fragments: Value<'v>) -> Self {
        Self { attr, fragments }
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
            attr: self.attr.freeze(freezer)?,
            fragments: self.fragments.freeze(freezer)?,
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
    attr: FrozenValue,
    #[allocative(skip)]
    fragments: FrozenValue,
}

unsafe impl<'v> Trace<'v> for FrozenFeatureContext {
    fn trace(&mut self, _tracer: &Tracer<'v>) {}
}

#[starlark_value(type = "FeatureContext")]
impl<'v> StarlarkValue<'v> for FrozenFeatureContext {
    type Canonical = FeatureContext<'v>;
}

#[starlark_module]
fn feature_context_methods(builder: &mut MethodsBuilder) {
    /// The feature's own configured instance. Read field values via `ctx.attr.field_name`.
    #[starlark(attribute)]
    fn attr<'v>(this: Value<'v>) -> anyhow::Result<Value<'v>> {
        let ctx = this
            .downcast_ref_err::<FeatureContext>()
            .map_err(|e| anyhow::anyhow!("{}", e))?;
        Ok(ctx.attr)
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

    /// HTTP client for making requests during feature initialization.
    fn http<'v>(#[allow(unused)] this: Value<'v>) -> anyhow::Result<Http> {
        Ok(Http::new())
    }
}
