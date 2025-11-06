use allocative::Allocative;
use derive_more::Display;

use futures::future::BoxFuture;
use futures::FutureExt;
use starlark::environment::{Methods, MethodsBuilder, MethodsStatic};
use starlark::eval::Evaluator;
use starlark::starlark_module;
use starlark::typing::Ty;
use starlark::values::type_repr::StarlarkTypeRepr;
use starlark::values::{self, AllocValue, Heap, Trace, Tracer, UnpackValue, ValueLike};
use starlark::values::{starlark_value, NoSerialize, ProvidesStaticType};
use std::cell::RefCell;
use std::fmt::Debug;
use std::rc::Rc;

use crate::engine::store::AxlStore;

pub trait FutureAlloc: Send {
    fn alloc_value_fut<'v>(self: Box<Self>, heap: &'v Heap) -> values::Value<'v>;
}

impl StarlarkTypeRepr for Box<dyn FutureAlloc> {
    type Canonical = Self;

    fn starlark_type_repr() -> Ty {
        Ty::never()
    }
}

impl<'v> AllocValue<'v> for Box<dyn FutureAlloc> {
    fn alloc_value(self, heap: &'v Heap) -> values::Value<'v> {
        self.alloc_value_fut(heap)
    }
}
pub type FutOutput = Result<Box<dyn FutureAlloc>, anyhow::Error>;

#[derive(Display, Allocative, ProvidesStaticType, NoSerialize)]
#[display("Future")]
pub struct StarlarkFuture {
    #[allocative(skip)]
    inner: Rc<RefCell<Option<BoxFuture<'static, FutOutput>>>>,
}

impl StarlarkFuture {
    pub fn from_future<T: FutureAlloc + Send + 'static>(
        fut: impl Future<Output = Result<T, anyhow::Error>> + Send + 'static,
    ) -> Self {
        use futures::TryFutureExt;
        Self {
            inner: Rc::new(RefCell::new(Some(
                fut.map_ok_or_else(|e| Err(e), |r| Ok(Box::new(r) as Box<dyn FutureAlloc>))
                    .boxed(),
            ))),
        }
    }

    pub fn as_fut(&self) -> impl Future<Output = FutOutput> + Send + 'static {
        let inner = self.inner.replace(None);
        let r = inner
            .ok_or(anyhow::anyhow!("future has already been awaited"))
            .unwrap();

        r.into_future()
    }
}

impl Future for StarlarkFuture {
    type Output = FutOutput;

    fn poll(
        self: std::pin::Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
    ) -> std::task::Poll<Self::Output> {
        self.inner.take().unwrap().poll_unpin(cx)
    }
}

unsafe impl<'v> Trace<'v> for StarlarkFuture {
    fn trace(&mut self, _tracer: &Tracer<'v>) {}
}

impl Debug for StarlarkFuture {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Future").finish()
    }
}

impl<'v> AllocValue<'v> for StarlarkFuture {
    fn alloc_value(self, heap: &'v Heap) -> values::Value<'v> {
        heap.alloc_complex_no_freeze(self)
    }
}

impl<'v> UnpackValue<'v> for StarlarkFuture {
    type Error = anyhow::Error;

    fn unpack_value_impl(value: values::Value<'v>) -> Result<Option<Self>, Self::Error> {
        let fut = value.downcast_ref_err::<StarlarkFuture>()?;
        Ok(Some(Self {
            inner: fut.inner.clone(),
        }))
    }
}

#[starlark_value(type = "Future")]
impl<'v> values::StarlarkValue<'v> for StarlarkFuture {
    fn get_methods() -> Option<&'static Methods> {
        static RES: MethodsStatic = MethodsStatic::new();
        RES.methods(future_methods)
    }
}

#[starlark_module]
pub(crate) fn future_methods(registry: &mut MethodsBuilder) {
    fn block<'v>(
        #[allow(unused)] this: values::Value<'v>,
        eval: &mut Evaluator<'v, '_, '_>,
    ) -> starlark::Result<values::Value<'v>> {
        let store = AxlStore::from_eval(eval)?;
        let this = this.downcast_ref_err::<StarlarkFuture>()?;
        let fut = this
            .inner
            .replace(None)
            .ok_or(anyhow::anyhow!("future has already been awaited"))?;
        let value = store.rt.block_on(fut)?;
        Ok(value.alloc_value_fut(eval.heap()))
    }
}
