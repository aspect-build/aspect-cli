//! `__test_future(value = ..., error = ...)`: a future that resolves to
//! `value`, or fails with `error` as the top of a two-link chain whose root
//! is "root cause". Lets the tests exercise `catch()` without a network.

use starlark::environment::GlobalsBuilder;
use starlark::starlark_module;
use starlark::values::{Heap, Value};

use starlark::typing::Ty;
use starlark::values::type_repr::StarlarkTypeRepr;

use crate::engine::r#async::future::{FutureAlloc, FutureOf};

struct Resolved(String);

impl StarlarkTypeRepr for Resolved {
    type Canonical = Self;

    fn starlark_type_repr() -> Ty {
        Ty::string()
    }
}

impl FutureAlloc for Resolved {
    fn alloc_value_fut<'v>(self: Box<Self>, heap: Heap<'v>) -> Value<'v> {
        heap.alloc(self.0)
    }
}

#[starlark_module]
pub(super) fn register(globals: &mut GlobalsBuilder) {
    fn __test_future<'v>(
        #[starlark(require = named)] value: Option<String>,
        #[starlark(require = named)] error: Option<String>,
    ) -> anyhow::Result<FutureOf<'v, Resolved>> {
        Ok(FutureOf::from_future(async move {
            match error {
                Some(msg) => Err(anyhow::anyhow!("root cause").context(msg)),
                None => Ok(Resolved(value.unwrap_or_default())),
            }
        }))
    }
}
