use allocative::Allocative;
use derive_more::Display;

use futures::FutureExt;
use futures::future::BoxFuture;
use starlark::StarlarkResultExt;
use starlark::environment::{Methods, MethodsBuilder, MethodsStatic};
use starlark::eval::Evaluator;
use starlark::starlark_module;
use starlark::typing::{ParamSpec, Ty, TyStarlarkValue, TyUser, TyUserFields, TyUserParams};
use starlark::values::type_repr::StarlarkTypeRepr;
use starlark::values::typing::TypeInstanceId;
use starlark::values::{self, AllocValue, Heap, Trace, Tracer, UnpackValue, ValueLike};
use starlark::values::{NoSerialize, ProvidesStaticType, starlark_value};
use std::cell::RefCell;
use std::collections::HashMap;
use std::fmt::Debug;
use std::marker::PhantomData;
use std::sync::{LazyLock, Mutex};

use crate::engine::error::{
    callback_error, caught_by, caught_ty, check_catch_types, err_pair, error_value_of, is_exit,
    ok_pair,
};
use crate::engine::store::Env;

pub trait FutureAlloc: Send {
    fn alloc_value_fut<'v>(self: Box<Self>, heap: Heap<'v>) -> values::Value<'v>;
}

impl StarlarkTypeRepr for Box<dyn FutureAlloc> {
    type Canonical = Self;

    fn starlark_type_repr() -> Ty {
        Ty::never()
    }
}

impl<'v> AllocValue<'v> for Box<dyn FutureAlloc> {
    fn alloc_value(self, heap: Heap<'v>) -> values::Value<'v> {
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
    inner: RefCell<Option<BoxFuture<'static, FutOutput>>>,
    #[allocative(skip)]
    transforms: RefCell<Vec<Transform<'v>>>,
    /// The error types `catch(...)` turns into values; empty catches every
    /// error. `None` until `catch` is called.
    #[allocative(skip)]
    catch: RefCell<Option<Vec<values::Value<'v>>>>,
}

impl<'v> StarlarkFuture<'v> {
    pub fn from_future<T: FutureAlloc + Send + 'static>(
        fut: impl Future<Output = Result<T, anyhow::Error>> + Send + 'static,
    ) -> Self {
        use futures::TryFutureExt;
        Self {
            inner: RefCell::new(Some(
                fut.map_ok_or_else(|e| Err(e), |r| Ok(Box::new(r) as Box<dyn FutureAlloc>))
                    .boxed(),
            )),
            transforms: RefCell::new(Vec::new()),
            catch: RefCell::new(None),
        }
    }

    pub fn as_fut(&self) -> impl Future<Output = FutOutput> + Send + 'static {
        let inner = self.inner.borrow_mut().take();
        let r = inner
            .ok_or(anyhow::anyhow!("future has already been awaited"))
            .unwrap();

        r.into_future()
    }

    fn with_transform(&self, transform: Transform<'v>) -> anyhow::Result<Self> {
        if self.catch.borrow().is_some() {
            anyhow::bail!("catch() must be the last call before block()");
        }
        let mut new_transforms = self.transforms.borrow().clone();
        new_transforms.push(transform);
        Ok(Self {
            // Move the inner future to the new chained future, consuming the original.
            inner: RefCell::new(self.inner.borrow_mut().take()),
            transforms: RefCell::new(new_transforms),
            catch: RefCell::new(None),
        })
    }

    fn with_catch(&self, types: Vec<values::Value<'v>>) -> anyhow::Result<Self> {
        if self.catch.borrow().is_some() {
            anyhow::bail!("catch() was already called on this future");
        }
        Ok(Self {
            inner: RefCell::new(self.inner.borrow_mut().take()),
            transforms: RefCell::new(self.transforms.borrow().clone()),
            catch: RefCell::new(Some(types)),
        })
    }
}

/// A future that resolves to a `T`, typed as `Future[T]`: the typechecker
/// knows what its `block()` returns and what `catch().block()` pairs. The
/// value is an ordinary `Future`; only the type is more precise.
pub struct FutureOf<'v, T>(StarlarkFuture<'v>, PhantomData<fn() -> T>);

impl<'v, T: FutureAlloc + 'static> FutureOf<'v, T> {
    pub fn from_future(
        fut: impl Future<Output = Result<T, anyhow::Error>> + Send + 'static,
    ) -> Self {
        Self(StarlarkFuture::from_future(fut), PhantomData)
    }
}

impl<T: StarlarkTypeRepr> StarlarkTypeRepr for FutureOf<'_, T> {
    type Canonical = Self;

    fn starlark_type_repr() -> Ty {
        future_ty(T::starlark_type_repr(), true)
    }
}

impl<'v, T: StarlarkTypeRepr> AllocValue<'v> for FutureOf<'v, T> {
    fn alloc_value(self, heap: Heap<'v>) -> values::Value<'v> {
        self.0.alloc_value(heap)
    }
}

/// `Future[T]` for a `value` of type `T`: `block()` returns `T`, and, while
/// `catchable`, `catch()` returns the `Future` whose `block()` gives the
/// `(err, value)` pair. Other methods are typed as for any `Future`.
fn future_ty(value: Ty, catchable: bool) -> Ty {
    // One identity per spelling, so every `Future[HttpResponse]` is the same type.
    static IDS: LazyLock<Mutex<HashMap<String, TypeInstanceId>>> = LazyLock::new(Default::default);

    let name = format!("Future[{value}]");
    let id = *IDS
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .entry(name.clone())
        .or_insert_with(TypeInstanceId::r#gen);
    let mut known = vec![(
        "block".to_owned(),
        Ty::callable(ParamSpec::empty(), value.clone()),
    )];
    if catchable {
        let types = ParamSpec::new_parts([], [], Some(Ty::any()), [], None)
            .expect("`*types` alone is a valid signature");
        known.push((
            "catch".to_owned(),
            Ty::callable(types, future_ty(caught_ty(value), false)),
        ));
    }
    TyUser::new(
        name,
        TyStarlarkValue::new::<StarlarkFuture>(),
        id,
        TyUserParams {
            fields: TyUserFields {
                known: known.into_iter().collect(),
                unknown: true,
            },
            ..TyUserParams::default()
        },
    )
    .map(Ty::custom)
    .unwrap_or_else(|_| Ty::starlark_value::<StarlarkFuture>())
}

impl<'v> Future for StarlarkFuture<'v> {
    type Output = FutOutput;

    fn poll(
        self: std::pin::Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
    ) -> std::task::Poll<Self::Output> {
        self.inner.borrow_mut().take().unwrap().poll_unpin(cx)
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
        if let Some(types) = self.catch.borrow_mut().as_mut() {
            for t in types.iter_mut() {
                t.trace(tracer);
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
    fn alloc_value(self, heap: Heap<'v>) -> values::Value<'v> {
        heap.alloc_complex_no_freeze(self)
    }
}

impl<'v> UnpackValue<'v> for StarlarkFuture<'v> {
    type Error = anyhow::Error;

    fn unpack_value_impl(value: values::Value<'v>) -> Result<Option<Self>, Self::Error> {
        let fut = value
            .downcast_ref_err::<StarlarkFuture>()
            .into_anyhow_result()?;
        Ok(Some(Self {
            // Move the inner future out of the original, consuming it.
            inner: RefCell::new(fut.inner.borrow_mut().take()),
            transforms: RefCell::new(fut.transforms.borrow().clone()),
            catch: RefCell::new(fut.catch.borrow().clone()),
        }))
    }
}

#[starlark_value(type = "Future")]
impl<'v> values::StarlarkValue<'v> for StarlarkFuture<'v> {
    fn get_methods() -> Option<&'static Methods> {
        static RES: MethodsStatic = MethodsStatic::new("future_methods", future_methods);
        Some(RES.methods())
    }
}

fn apply_transforms<'v>(
    result: FutOutput,
    transforms: &[Transform<'v>],
    eval: &mut Evaluator<'v, '_, '_>,
) -> anyhow::Result<values::Value<'v>> {
    let heap = eval.heap();
    let mut current: Result<values::Value<'v>, anyhow::Error> =
        result.map(|boxed| boxed.alloc_value_fut(heap));

    for transform in transforms {
        current = match (current, transform) {
            (Ok(val), Transform::MapOk(f)) => {
                eval.eval_function(*f, &[val], &[]).map_err(callback_error)
            }
            (Err(e), Transform::MapOk(_)) => Err(e),

            (Err(e), Transform::MapErr(f)) => {
                let err_str = heap.alloc_str(&e.to_string()).to_value();
                eval.eval_function(*f, &[err_str], &[])
                    .map_err(callback_error)
            }
            (Ok(v), Transform::MapErr(_)) => Ok(v),

            (Ok(val), Transform::MapOkOrElse { map_ok, .. }) => eval
                .eval_function(*map_ok, &[val], &[])
                .map_err(callback_error),
            (Err(e), Transform::MapOkOrElse { map_err, .. }) => {
                let err_str = heap.alloc_str(&e.to_string()).to_value();
                eval.eval_function(*map_err, &[err_str], &[])
                    .map_err(callback_error)
            }
        };
    }

    current
}

#[starlark_module]
pub(crate) fn future_methods(registry: &mut MethodsBuilder) {
    fn block<'v>(
        this: values::Value<'v>,
        eval: &mut Evaluator<'v, '_, '_>,
    ) -> anyhow::Result<values::Value<'v>> {
        let env = Env::from_eval(eval)?;
        let this = this
            .downcast_ref_err::<StarlarkFuture>()
            .into_anyhow_result()?;

        let fut = this
            .inner
            .borrow_mut()
            .take()
            .ok_or(anyhow::anyhow!("future has already been awaited"))?;
        let transforms = this.transforms.borrow().clone();

        let catch = this.catch.borrow().clone();

        let result = env.rt.block_on(fut);
        let result = apply_transforms(result, &transforms, eval);
        let Some(types) = catch else {
            return result;
        };
        match result {
            Ok(value) => Ok(ok_pair(value, eval.heap())),
            Err(err) if is_exit(&err) => Err(err),
            Err(err) => {
                let error = error_value_of(&err, eval);
                if caught_by(&types, error) {
                    Ok(err_pair(error, eval.heap()))
                } else {
                    Err(err)
                }
            }
        }
    }

    /// Turn the future's failure into a value instead of an error.
    ///
    /// `block()` on the returned future gives an `(err, value)` pair:
    /// `(None, value)` when it succeeds, `(err, None)` when it fails with an
    /// error of one of `types`. With no `types`, every failure is caught. A
    /// failure of any other type still raises, unchanged, and so does a
    /// `ctx.std.process.exit` in a `map_ok` callback.
    ///
    /// A failure raised by a runtime operation arrives as a plain `error`:
    /// its `message` is the failure, its `cause` chain holds the underlying
    /// reasons, and its `stacktrace` points at the `block()` call. An error
    /// value raised with `fail(e)` in a `map_ok` callback arrives as that
    /// same value.
    ///
    /// `catch` must be the last call before `block()`.
    ///
    /// ```starlark
    /// err, resp = ctx.http().get(url = url).catch().block()
    /// if err:
    ///     print("request failed: " + err.message)
    ///     return 1
    /// print(resp.status)
    /// ```
    fn catch<'v>(
        this: values::Value<'v>,
        #[starlark(args)] types: starlark::values::tuple::UnpackTuple<values::Value<'v>>,
    ) -> anyhow::Result<StarlarkFuture<'v>> {
        let this_fut = this
            .downcast_ref_err::<StarlarkFuture>()
            .into_anyhow_result()?;
        check_catch_types(&types.items)?;
        this_fut.with_catch(types.items)
    }

    fn map_ok<'v>(
        this: values::Value<'v>,
        callable: values::Value<'v>,
    ) -> anyhow::Result<StarlarkFuture<'v>> {
        let this_fut = this
            .downcast_ref_err::<StarlarkFuture>()
            .into_anyhow_result()?;
        this_fut.with_transform(Transform::MapOk(callable))
    }

    fn map_err<'v>(
        this: values::Value<'v>,
        callable: values::Value<'v>,
    ) -> anyhow::Result<StarlarkFuture<'v>> {
        let this_fut = this
            .downcast_ref_err::<StarlarkFuture>()
            .into_anyhow_result()?;
        this_fut.with_transform(Transform::MapErr(callable))
    }

    fn map_ok_or_else<'v>(
        this: values::Value<'v>,
        #[starlark(require = named)] map_ok: values::Value<'v>,
        #[starlark(require = named)] map_err: values::Value<'v>,
    ) -> anyhow::Result<StarlarkFuture<'v>> {
        let this_fut = this
            .downcast_ref_err::<StarlarkFuture>()
            .into_anyhow_result()?;
        this_fut.with_transform(Transform::MapOkOrElse { map_ok, map_err })
    }
}
