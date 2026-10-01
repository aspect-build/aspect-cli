//! Turning a failure back into an error value: the `catch` global, and the
//! pieces `future.catch()` shares with it.
//!
//! Both hand back an `(err, value)` pair and leave two kinds of failure
//! alone: an error whose type is not among the requested ones, which raises
//! unchanged, and a `ctx.std.process.exit`, which ends the task as asked.

use allocative::Allocative;
use pagable::Pagable;
use starlark::codemap::Span;
use starlark::environment::GlobalsBuilder;
use starlark::eval::{CallStack, Evaluator};
use starlark::starlark_module;
use starlark::typing::{
    ParamIsRequired, ParamSpec, Ty, TyCallArgs, TyCallable, TyCustomFunctionImpl,
    TypingOrInternalError, TypingOracleCtx,
};
use starlark::util::ArcStr;
use starlark::values::list_or_tuple::UnpackListOrTuple;
use starlark::values::tuple::{AllocTuple, UnpackTuple};
use starlark::values::type_repr::StarlarkTypeRepr;
use starlark::values::{AllocValue, Heap, StringValue, Value};
use starlark_map::small_map::SmallMap;

use super::error_type::{error_instance_ty, error_type_id};
use super::native::NativeRaised;
use super::raise::RaisedError;
use super::value::{ErrorValue, ErrorValueRef};
use crate::eval::TaskExit;

/// `err` as an error value on the evaluator's heap: the value itself when it
/// was raised with `fail(e)`, or else a plain `error` whose message is the top
/// of the chain and whose `cause` holds the rest, one link per `source()`.
pub(crate) fn error_value_of<'v>(
    err: &anyhow::Error,
    eval: &mut Evaluator<'v, '_, '_>,
) -> Value<'v> {
    if let Some(value) = RaisedError::from_anyhow(err).and_then(|r| r.value(eval)) {
        return value;
    }
    if let Some(native) = NativeRaised::from_anyhow(err) {
        return native.to_value(eval);
    }
    let links = err.chain().map(ToString::to_string).collect();
    let stack = eval.call_stack();
    plain_error(links, &stack, eval.heap())
}

/// A Starlark error as an error value, as [`error_value_of`] does for an
/// `anyhow` one. The stacktrace is where the error was raised.
fn starlark_error_value_of<'v>(
    err: &starlark::Error,
    eval: &mut Evaluator<'v, '_, '_>,
) -> Value<'v> {
    if let Some(value) = RaisedError::from_starlark(err).and_then(|r| r.value(eval)) {
        return value;
    }
    if let Some(native) = NativeRaised::from_starlark(err) {
        return native.to_value(eval);
    }
    let links = match err.kind() {
        starlark::ErrorKind::Native(e)
        | starlark::ErrorKind::Other(e)
        | starlark::ErrorKind::Value(e) => e.chain().map(ToString::to_string).collect(),
        // Starlark's `fail` prefixes its message with a space.
        starlark::ErrorKind::Fail(e) => vec![e.to_string().trim_start().to_owned()],
        _ => vec![err.without_diagnostic().to_string()],
    };
    let stack = if err.call_stack().is_empty() {
        eval.call_stack()
    } else {
        err.call_stack().clone()
    };
    plain_error(links, &stack, eval.heap())
}

/// A chain of plain errors, one per message, the first outermost.
fn plain_error<'v>(links: Vec<String>, stack: &CallStack, heap: Heap<'v>) -> Value<'v> {
    links
        .into_iter()
        .rev()
        .fold(Value::new_none(), |cause, message| {
            heap.alloc_complex(ErrorValue {
                typ: Value::new_none(),
                message,
                cause,
                values: Box::new([]),
                stack: stack.clone(),
            })
        })
}

/// Whether `error` is one `catch(*types)` asked for: any error when `types`
/// is empty, else an instance of one of them.
pub(crate) fn caught_by(types: &[Value], error: Value) -> bool {
    types.is_empty()
        || ErrorValueRef::of(error).is_some_and(|e| {
            types
                .iter()
                .filter_map(|t| error_type_id(*t))
                .any(|id| e.is_instance_of(id))
        })
}

/// Check that every one of `types` is an error type.
pub(crate) fn check_catch_types(types: &[Value]) -> anyhow::Result<()> {
    for t in types {
        if error_type_id(*t).is_none() {
            anyhow::bail!(
                "catch() takes error types, got `{}` of type `{}`",
                t,
                t.get_type()
            );
        }
    }
    Ok(())
}

/// Whether `err` is a `ctx.std.process.exit` (or a Rust refusal), which
/// ends the task rather than being caught. A raised error value is not one,
/// even when its type renders without a traceback.
pub(crate) fn is_exit(err: &anyhow::Error) -> bool {
    err.downcast_ref::<TaskExit>().is_some()
}

fn is_starlark_exit(err: &starlark::Error) -> bool {
    matches!(
        err.kind(),
        starlark::ErrorKind::Native(e) | starlark::ErrorKind::Other(e) if is_exit(e)
    )
}

/// The `(err, value)` pair a successful call produces.
pub(crate) fn ok_pair<'v>(value: Value<'v>, heap: Heap<'v>) -> Value<'v> {
    heap.alloc(AllocTuple([Value::new_none(), value]))
}

/// The `(err, value)` pair a caught failure produces.
pub(crate) fn err_pair<'v>(error: Value<'v>, heap: Heap<'v>) -> Value<'v> {
    heap.alloc(AllocTuple([error, Value::new_none()]))
}

/// A callback's error as an `anyhow::Error`, keeping a raised error value
/// and an exit intact so a later `catch()` can still tell them apart.
/// Anything else is flattened to its rendering, as before.
pub(crate) fn callback_error(err: starlark::Error) -> anyhow::Error {
    if RaisedError::from_starlark(&err).is_some() || is_starlark_exit(&err) {
        match err.into_kind() {
            starlark::ErrorKind::Native(e) | starlark::ErrorKind::Other(e) => return e,
            _ => unreachable!("both only match Native and Other"),
        }
    }
    anyhow::anyhow!("{}", err)
}

/// The type of an `(err, value)` pair whose value, on success, is `value`.
pub(crate) fn caught_ty(value: Ty) -> Ty {
    caught_ty_with(error_instance_ty(), value)
}

/// The type of an `(err, value)` pair that only catches errors typed `error`.
pub(crate) fn caught_ty_with(error: Ty, value: Ty) -> Ty {
    Ty::tuple2(Ty::union2(error, Ty::none()), Ty::union2(value, Ty::none()))
}

/// The `(err, value)` pair `catch` returns.
pub(crate) struct Caught<'v>(Value<'v>);

impl StarlarkTypeRepr for Caught<'_> {
    type Canonical = Self;

    fn starlark_type_repr() -> Ty {
        caught_ty(Ty::any())
    }
}

/// How the typechecker sees a `catch(function, *args, **kwargs)` call: the
/// call `function(*args, **kwargs)` it makes, checked as such, whose result
/// type becomes the pair's value type.
#[derive(Clone, Debug, Eq, PartialEq, Hash, Ord, PartialOrd, Allocative, Pagable)]
struct CatchType;

starlark::register_ty_custom_function!(CatchType);

impl TyCustomFunctionImpl for CatchType {
    fn as_callable(&self) -> TyCallable {
        let params = ParamSpec::new_parts(
            [(ParamIsRequired::Yes, Ty::any())],
            [],
            Some(Ty::any()),
            [(ArcStr::from("types"), ParamIsRequired::No, Ty::any())],
            Some(Ty::any()),
        )
        .expect("catch's parameters are distinct");
        TyCallable::new(params, caught_ty(Ty::any()))
    }

    fn validate_call(
        &self,
        span: Span,
        args: &TyCallArgs,
        oracle: TypingOracleCtx,
    ) -> Result<Ty, TypingOrInternalError> {
        let Some((function, rest)) = args.pos().split_first() else {
            return Err(oracle.msg_error(span, "catch() needs the function to call"));
        };
        let named = args
            .named()
            .iter()
            .filter(|arg| arg.node.0 != "types")
            .cloned()
            .collect();
        let call = TyCallArgs::new(
            rest.to_vec(),
            named,
            args.args().cloned(),
            args.kwargs().cloned(),
        );
        let result = oracle.validate_call(function.span, &function.node, &call)?;
        Ok(caught_ty(result))
    }
}

impl<'v> AllocValue<'v> for Caught<'v> {
    fn alloc_value(self, _heap: Heap<'v>) -> Value<'v> {
        self.0
    }
}

#[starlark_module]
pub(super) fn register_catch(globals: &mut GlobalsBuilder) {
    /// Call `function` and turn its failure into a value instead of an error.
    ///
    /// Returns an `(err, value)` pair: `(None, result)` when the call
    /// returns, `(err, None)` when it fails with an error of one of `types`.
    /// With no `types`, every failure is caught. A failure of any other type
    /// still raises, unchanged, and so does `ctx.std.process.exit`, which
    /// ends the task as asked.
    ///
    /// The remaining positional and keyword arguments are passed to
    /// `function`; `types` is `catch`'s own.
    ///
    /// An error raised with `fail(e)` arrives as `e`. Any other failure
    /// arrives as a plain `error` whose `message` is the failure, whose
    /// `cause` chain holds the underlying reasons, and whose `stacktrace`
    /// points at where it was raised.
    ///
    /// ```starlark
    /// err, config = catch(_load_config, ctx, path, types = [ConfigError])
    /// if err:
    ///     print("using defaults: " + err.message)
    ///     config = DEFAULT_CONFIG
    /// ```
    #[starlark(ty_custom_function = CatchType)]
    fn catch<'v>(
        #[starlark(require = pos)] function: Value<'v>,
        #[starlark(args)] args: UnpackTuple<Value<'v>>,
        #[starlark(require = named, default = UnpackListOrTuple::default())]
        types: UnpackListOrTuple<Value<'v>>,
        #[starlark(kwargs)] kwargs: SmallMap<StringValue<'v>, Value<'v>>,
        eval: &mut Evaluator<'v, '_, '_>,
    ) -> starlark::Result<Caught<'v>> {
        check_catch_types(&types.items).map_err(starlark::Error::new_other)?;
        let named: Vec<(&str, Value<'v>)> = kwargs.iter().map(|(k, v)| (k.as_str(), *v)).collect();
        match eval.eval_function(function, &args.items, &named) {
            Ok(value) => Ok(Caught(ok_pair(value, eval.heap()))),
            Err(err) if is_starlark_exit(&err) => Err(err),
            Err(err) => {
                let error = starlark_error_value_of(&err, eval);
                if caught_by(&types.items, error) {
                    Ok(Caught(err_pair(error, eval.heap())))
                } else {
                    Err(err)
                }
            }
        }
    }
}
