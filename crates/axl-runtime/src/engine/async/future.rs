use allocative::Allocative;
use derive_more::Display;

use futures::future::BoxFuture;
use futures::FutureExt;
use starlark::docs::{DocItem, DocMember, DocProperty, DocString};
use starlark::environment::{Methods, MethodsBuilder, MethodsStatic};
use starlark::eval::Evaluator;
use starlark::starlark_module;
use starlark::typing::{Ty, TyStarlarkValue, TyUser, TyUserParams};
use starlark::values::typing::TypeInstanceId;
use starlark::values::{self, AllocValue, Heap, Trace, Tracer, UnpackValue, ValueLike};
use starlark::values::{starlark_value, NoSerialize, ProvidesStaticType};
use std::cell::RefCell;
use std::fmt::Debug;
use std::rc::Rc;

use super::rt::AsyncRuntime;

pub trait FutureAlloc: Send {
    fn alloc_value_fut<'v>(self: Box<Self>, heap: &'v Heap) -> values::Value<'v>;
}

#[derive(Display, Allocative, ProvidesStaticType, NoSerialize)]
#[display("future")]
pub struct StarlarkFuture {
    #[allocative(skip)]
    inner: Rc<RefCell<Option<BoxFuture<'static, Box<dyn FutureAlloc>>>>>,
    tys: TyStarlarkValue,
    ty: Ty,
}

impl StarlarkFuture {
    pub fn from_future<'v, T: values::StarlarkValue<'v>>(
        fut: impl Future<Output = Box<dyn FutureAlloc>> + Send + 'static,
    ) -> Self {
        Self {
            inner: Rc::new(RefCell::new(Some(fut.boxed()))),
            tys: TyStarlarkValue::new::<T>(),
            ty: Ty::starlark_value::<T>(),
        }
    }
    pub fn as_fut(&self) -> impl Future<Output = Box<dyn FutureAlloc>> + Send + 'static {
        let inner = self.inner.replace(None);
        let r = inner
            .ok_or(anyhow::anyhow!("future has already been awaited"))
            .unwrap();

        r.into_future()
    }
}

impl Future for StarlarkFuture {
    type Output = Box<dyn FutureAlloc>;

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
        f.debug_struct("StarlarkFuture").finish()
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
            ty: fut.ty.clone(),
            tys: fut.tys,
        }))
    }
}

#[starlark_value(type = "future")]
impl<'v> values::StarlarkValue<'v> for StarlarkFuture {
    fn typechecker_ty(&self) -> Option<Ty> {
        let ty = TyUser::new(
            format!("future[{}]", self.ty.as_name()?),
            self.tys,
            TypeInstanceId::r#gen(),
            TyUserParams {
                // iter_item: Some(self.ty.clone()),
                ..Default::default()
            },
        )
        .ok()?;
        Some(Ty::custom(ty))
    }

    /// Evaluate this value as a type expression. Basically, `eval_type(this)`.
    fn eval_type(&self) -> Option<Ty> {
        self.typechecker_ty()
    }

    fn documentation(&self) -> DocItem
    where
        Self: Sized,
    {
        let ty = self
            .typechecker_ty()
            .unwrap_or_else(Self::get_type_starlark_repr);

        DocItem::Member(DocMember::Property(DocProperty {
            typ: ty,
            docs: DocString::from_docstring(starlark::docs::DocStringKind::Rust, "/// hello"),
        }))
    }

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
        let rt = AsyncRuntime::from_eval(eval)?;
        let this = this.downcast_ref_err::<StarlarkFuture>()?;
        let fut = this
            .inner
            .replace(None)
            .ok_or(anyhow::anyhow!("future has already been awaited"))?;
        let value = rt.block_on(fut);
        Ok(value.alloc_value_fut(eval.heap()))
    }
}
