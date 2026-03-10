pub mod any_registry;
pub mod delimited;

use prost_types::{Duration, Timestamp};

#[derive(
    ::starlark::values::ProvidesStaticType,
    ::derive_more::Display,
    ::starlark::values::Trace,
    ::starlark::values::NoSerialize,
    ::allocative::Allocative,
    Debug,
)]
#[display("Duration")]
pub struct SBDuration(#[allocative(skip)] pub Duration);

#[starlark::values::starlark_value(type = "duration")]
impl<'v> starlark::values::StarlarkValue<'v> for SBDuration {}

#[derive(
    ::starlark::values::ProvidesStaticType,
    ::derive_more::Display,
    ::starlark::values::Trace,
    ::starlark::values::NoSerialize,
    ::allocative::Allocative,
    Debug,
)]
#[display("Timestamp")]
pub struct SBTimestamp(#[allocative(skip)] pub Timestamp);

#[starlark::values::starlark_value(type = "Timestamp")]
impl<'v> starlark::values::StarlarkValue<'v> for SBTimestamp {}

#[derive(
    ::starlark::values::ProvidesStaticType,
    ::starlark::values::Trace,
    ::starlark::values::NoSerialize,
    ::allocative::Allocative,
    Debug,
)]
pub struct SBAny {
    #[allocative(skip)]
    pub type_url: String,
    #[allocative(skip)]
    pub value: Vec<u8>,
}

impl SBAny {
    pub fn from_prost(any: &prost_types::Any) -> Self {
        SBAny {
            type_url: any.type_url.clone(),
            value: any.value.clone(),
        }
    }
}

impl std::fmt::Display for SBAny {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "Any({})", self.type_url)
    }
}

impl<'v> starlark::values::AllocValue<'v> for SBAny {
    fn alloc_value(self, heap: starlark::values::Heap<'v>) -> starlark::values::Value<'v> {
        heap.alloc_simple(self)
    }
}

#[starlark::values::starlark_value(type = "Any")]
impl<'v> starlark::values::StarlarkValue<'v> for SBAny {
    fn get_methods() -> Option<&'static starlark::environment::Methods> {
        static RES: starlark::environment::MethodsStatic =
            starlark::environment::MethodsStatic::new();
        RES.methods(sb_any_methods)
    }
}

#[starlark::starlark_module]
fn sb_any_methods(registry: &mut starlark::environment::MethodsBuilder) {
    #[starlark(attribute)]
    fn type_url<'v>(this: starlark::values::Value<'v>) -> anyhow::Result<&'v str> {
        use starlark::StarlarkResultExt;
        use starlark::values::ValueLike;
        let this = this.downcast_ref_err::<SBAny>().into_anyhow_result()?;
        Ok(&this.type_url)
    }

    fn value<'v>(
        this: starlark::values::Value<'v>,
        heap: starlark::values::Heap<'v>,
    ) -> anyhow::Result<starlark::values::Value<'v>> {
        use starlark::StarlarkResultExt;
        use starlark::values::ValueLike;
        let this = this.downcast_ref_err::<SBAny>().into_anyhow_result()?;
        Ok(heap.alloc(starlark::values::bytes::StarlarkBytes::new(&this.value)))
    }

    fn unpack<'v>(
        this: starlark::values::Value<'v>,
        heap: starlark::values::Heap<'v>,
    ) -> anyhow::Result<starlark::values::Value<'v>> {
        use starlark::StarlarkResultExt;
        use starlark::values::ValueLike;
        let this = this.downcast_ref_err::<SBAny>().into_anyhow_result()?;
        any_registry::unpack(&this.type_url, &this.value, heap)
    }
}
