use allocative::Allocative;
use derive_more::Display;

use futures::FutureExt;
use futures::future::BoxFuture;
use starlark::environment::{Methods, MethodsBuilder, MethodsStatic};
use starlark::eval::Evaluator;
use starlark::starlark_module;
use starlark::typing::Ty;
use starlark::values::type_repr::StarlarkTypeRepr;
use starlark::values::{self, AllocValue, Heap, Trace, Tracer, UnpackValue, ValueLike};
use starlark::values::{NoSerialize, ProvidesStaticType, starlark_value};
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

#[derive(Clone, Copy)]
pub enum Transform<'v> {
    MapOk(values::Value<'v>),
    MapErr(values::Value<'v>),
    MapOkOrElse {
        map_ok: values::Value<'v>,
        map_err: values::Value<'v>,
    },
}

#[derive(Display, Allocative, ProvidesStaticType, NoSerialize)]
#[display("Future")]
pub struct StarlarkFuture<'v> {
    #[allocative(skip)]
    inner: Rc<RefCell<Option<BoxFuture<'static, FutOutput>>>>,
    #[allocative(skip)]
    transforms: Rc<RefCell<Vec<Transform<'v>>>>,
}

impl<'v> StarlarkFuture<'v> {
    pub fn from_future<T: FutureAlloc + Send + 'static>(
        fut: impl Future<Output = Result<T, anyhow::Error>> + Send + 'static,
    ) -> Self {
        use futures::TryFutureExt;
        Self {
            inner: Rc::new(RefCell::new(Some(
                fut.map_ok_or_else(|e| Err(e), |r| Ok(Box::new(r) as Box<dyn FutureAlloc>))
                    .boxed(),
            ))),
            transforms: Rc::new(RefCell::new(Vec::new())),
        }
    }

    pub fn as_fut(&self) -> impl Future<Output = FutOutput> + Send + 'static {
        let inner = self.inner.replace(None);
        let r = inner
            .ok_or(anyhow::anyhow!("future has already been awaited"))
            .unwrap();

        r.into_future()
    }

    fn with_transform(&self, transform: Transform<'v>) -> Self {
        let mut new_transforms = self.transforms.borrow().clone();
        new_transforms.push(transform);
        Self {
            inner: self.inner.clone(),
            transforms: Rc::new(RefCell::new(new_transforms)),
        }
    }
}

impl<'v> Future for StarlarkFuture<'v> {
    type Output = FutOutput;

    fn poll(
        self: std::pin::Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
    ) -> std::task::Poll<Self::Output> {
        self.inner.take().unwrap().poll_unpin(cx)
    }
}

unsafe impl<'v> Trace<'v> for StarlarkFuture<'v> {
    fn trace(&mut self, tracer: &Tracer<'v>) {
        for transform in self.transforms.borrow_mut().iter_mut() {
            match transform {
                Transform::MapOk(v) => v.trace(tracer),
                Transform::MapErr(v) => v.trace(tracer),
                Transform::MapOkOrElse { map_ok, map_err } => {
                    map_ok.trace(tracer);
                    map_err.trace(tracer);
                }
            }
        }
    }
}

impl<'v> Debug for StarlarkFuture<'v> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Future").finish()
    }
}

impl<'v> AllocValue<'v> for StarlarkFuture<'v> {
    fn alloc_value(self, heap: &'v Heap) -> values::Value<'v> {
        heap.alloc_complex_no_freeze(self)
    }
}

impl<'v> UnpackValue<'v> for StarlarkFuture<'v> {
    type Error = anyhow::Error;

    fn unpack_value_impl(value: values::Value<'v>) -> Result<Option<Self>, Self::Error> {
        let fut = value.downcast_ref_err::<StarlarkFuture>()?;
        Ok(Some(Self {
            inner: fut.inner.clone(),
            transforms: fut.transforms.clone(),
        }))
    }
}

#[starlark_value(type = "Future")]
impl<'v> values::StarlarkValue<'v> for StarlarkFuture<'v> {
    fn get_methods() -> Option<&'static Methods> {
        static RES: MethodsStatic = MethodsStatic::new();
        RES.methods(future_methods)
    }
}

fn apply_transforms<'v>(
    result: FutOutput,
    transforms: &[Transform<'v>],
    eval: &mut Evaluator<'v, '_, '_>,
) -> starlark::Result<values::Value<'v>> {
    let heap = eval.heap();
    let mut current: Result<values::Value<'v>, anyhow::Error> =
        result.map(|boxed| boxed.alloc_value_fut(heap));

    for transform in transforms {
        current = match (current, transform) {
            (Ok(val), Transform::MapOk(f)) => eval
                .eval_function(*f, &[val], &[])
                .map_err(|e| anyhow::anyhow!("{}", e)),
            (Err(e), Transform::MapOk(_)) => Err(e),

            (Err(e), Transform::MapErr(f)) => {
                let err_str = heap.alloc_str(&e.to_string()).to_value();
                eval.eval_function(*f, &[err_str], &[])
                    .map_err(|e| anyhow::anyhow!("{}", e))
            }
            (Ok(v), Transform::MapErr(_)) => Ok(v),

            (Ok(val), Transform::MapOkOrElse { map_ok, .. }) => eval
                .eval_function(*map_ok, &[val], &[])
                .map_err(|e| anyhow::anyhow!("{}", e)),
            (Err(e), Transform::MapOkOrElse { map_err, .. }) => {
                let err_str = heap.alloc_str(&e.to_string()).to_value();
                eval.eval_function(*map_err, &[err_str], &[])
                    .map_err(|e| anyhow::anyhow!("{}", e))
            }
        };
    }

    current.map_err(|e| starlark::Error::from(anyhow::anyhow!("{}", e)))
}

#[starlark_module]
pub(crate) fn future_methods(registry: &mut MethodsBuilder) {
    fn block<'v>(
        this: values::Value<'v>,
        eval: &mut Evaluator<'v, '_, '_>,
    ) -> starlark::Result<values::Value<'v>> {
        let store = AxlStore::from_eval(eval)?;
        let this = this.downcast_ref_err::<StarlarkFuture>()?;

        let fut = this
            .inner
            .replace(None)
            .ok_or(anyhow::anyhow!("future has already been awaited"))?;
        let transforms = this.transforms.borrow().clone();

        let result = store.rt.block_on(fut);
        apply_transforms(result, &transforms, eval)
    }

    fn map_ok<'v>(
        this: values::Value<'v>,
        callable: values::Value<'v>,
    ) -> starlark::Result<StarlarkFuture<'v>> {
        let this_fut = this.downcast_ref_err::<StarlarkFuture>()?;
        Ok(this_fut.with_transform(Transform::MapOk(callable)))
    }

    fn map_err<'v>(
        this: values::Value<'v>,
        callable: values::Value<'v>,
    ) -> starlark::Result<StarlarkFuture<'v>> {
        let this_fut = this.downcast_ref_err::<StarlarkFuture>()?;
        Ok(this_fut.with_transform(Transform::MapErr(callable)))
    }

    fn map_ok_or_else<'v>(
        this: values::Value<'v>,
        #[starlark(require = named)] map_ok: values::Value<'v>,
        #[starlark(require = named)] map_err: values::Value<'v>,
    ) -> starlark::Result<StarlarkFuture<'v>> {
        let this_fut = this.downcast_ref_err::<StarlarkFuture>()?;
        Ok(this_fut.with_transform(Transform::MapOkOrElse { map_ok, map_err }))
    }
}
