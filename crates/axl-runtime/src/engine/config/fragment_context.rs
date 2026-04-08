use allocative::Allocative;
use derive_more::Display;

use starlark::environment::{Methods, MethodsBuilder, MethodsStatic};
use starlark::starlark_module;
use starlark::values::{
    AllocValue, Freeze, FreezeError, Freezer, FrozenValue, Heap, NoSerialize, ProvidesStaticType,
    StarlarkValue, Trace, Tracer, Value, ValueLike, starlark_value,
};

use crate::engine::std::Std;

/// Context passed to a fragment's default-value function.
///
/// `ctx.attr` provides mutable access to the fragment instance so the function
/// can read and set field values.  `ctx.std` exposes the same standard library
/// available in `ConfigContext`.
#[derive(Debug, ProvidesStaticType, NoSerialize, Allocative, Display)]
#[display("<FragmentContext>")]
pub struct FragmentContext<'v> {
    /// The fragment instance whose fields the default function may read/write.
    #[allocative(skip)]
    pub(crate) attr: Value<'v>,
}

unsafe impl<'v> Trace<'v> for FragmentContext<'v> {
    fn trace(&mut self, tracer: &Tracer<'v>) {
        self.attr.trace(tracer);
    }
}

impl<'v> FragmentContext<'v> {
    pub fn new(attr: Value<'v>) -> Self {
        Self { attr }
    }
}

impl<'v> AllocValue<'v> for FragmentContext<'v> {
    fn alloc_value(self, heap: Heap<'v>) -> Value<'v> {
        heap.alloc_complex(self)
    }
}

impl<'v> Freeze for FragmentContext<'v> {
    type Frozen = FrozenFragmentContext;

    fn freeze(self, freezer: &Freezer) -> Result<Self::Frozen, FreezeError> {
        Ok(FrozenFragmentContext {
            attr: self.attr.freeze(freezer)?,
        })
    }
}

#[starlark_value(type = "FragmentContext")]
impl<'v> StarlarkValue<'v> for FragmentContext<'v> {
    fn get_methods() -> Option<&'static Methods> {
        static RES: MethodsStatic = MethodsStatic::new();
        RES.methods(fragment_context_methods)
    }
}

/// Frozen version of FragmentContext (not used in practice).
#[derive(Debug, ProvidesStaticType, NoSerialize, Allocative, Display)]
#[display("<FragmentContext>")]
pub struct FrozenFragmentContext {
    #[allocative(skip)]
    attr: FrozenValue,
}

unsafe impl<'v> Trace<'v> for FrozenFragmentContext {
    fn trace(&mut self, _tracer: &Tracer<'v>) {}
}

#[starlark_value(type = "FragmentContext")]
impl<'v> StarlarkValue<'v> for FrozenFragmentContext {
    type Canonical = FragmentContext<'v>;
}

#[starlark_module]
fn fragment_context_methods(builder: &mut MethodsBuilder) {
    /// The fragment instance. Use `ctx.attr.field_name` to read or set fields.
    #[starlark(attribute)]
    fn attr<'v>(this: Value<'v>) -> anyhow::Result<Value<'v>> {
        let ctx = this
            .downcast_ref_err::<FragmentContext>()
            .map_err(|e| anyhow::anyhow!("{}", e))?;
        Ok(ctx.attr)
    }

    /// Standard library — same as `ctx.std` in config functions.
    #[starlark(attribute)]
    fn std<'v>(#[allow(unused)] this: Value<'v>) -> anyhow::Result<Std> {
        Ok(Std {})
    }
}
