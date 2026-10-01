//! Raising an error value: the `fail` that takes one, and the payload that
//! carries it through the evaluator.

use std::cell::RefCell;
use std::fmt::{self, Display};
use std::sync::atomic::{AtomicU64, Ordering};

use allocative::Allocative;
use starlark::environment::GlobalsBuilder;
use starlark::eval::Evaluator;
use starlark::starlark_module;
use starlark::starlark_simple_value;
use starlark::values::tuple::UnpackTuple;
use starlark::values::typing::StarlarkNever;
use starlark::values::{
    Freeze, FreezeResult, Freezer, NoSerialize, ProvidesStaticType, StarlarkValue, Trace, Value,
    ValueLike, starlark_value,
};

use crate::eval::{EvalError, TaskExit};

use super::value::ErrorValueRef;

/// The raised error values of one module, kept live on its heap.
///
/// A raised value cannot ride inside an `anyhow::Error`, which must be
/// `'static`, and freezing it would forward the live value (and every value
/// its fields reach) out from under the code still holding it. So `fail(e)`
/// parks `e` here, in the module's traced `extra_value` slot where the
/// garbage collector sees it, and the error carries only the slot's index.
#[derive(Debug, Trace, ProvidesStaticType, NoSerialize, Allocative)]
struct RaisedValues<'v> {
    #[trace(static)]
    id: u64,
    #[allocative(skip)]
    values: RefCell<Vec<Value<'v>>>,
}

static RAISED_VALUES_ID: AtomicU64 = AtomicU64::new(0);

impl Display for RaisedValues<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("raised_values")
    }
}

#[starlark_value(type = "raised_values")]
impl<'v> StarlarkValue<'v> for RaisedValues<'v> {}

/// A module that finishes loading keeps none of its raised values.
#[derive(Debug, ProvidesStaticType, NoSerialize, Allocative, derive_more::Display)]
#[display("raised_values")]
struct FrozenRaisedValues;

starlark_simple_value!(FrozenRaisedValues);

#[starlark_value(type = "raised_values")]
impl<'v> StarlarkValue<'v> for FrozenRaisedValues {
    type Canonical = RaisedValues<'v>;
}

impl Freeze for RaisedValues<'_> {
    type Frozen = FrozenRaisedValues;

    fn freeze(self, _freezer: &Freezer) -> FreezeResult<Self::Frozen> {
        Ok(FrozenRaisedValues)
    }
}

impl<'v> RaisedValues<'v> {
    /// The current module's store, created on first use. `None` if the slot
    /// already holds something else.
    fn of(eval: &Evaluator<'v, '_, '_>) -> Option<&'v RaisedValues<'v>> {
        let module = eval.module();
        let slot = match module.extra_value() {
            Some(v) => v,
            None => {
                let v = eval.heap().alloc_complex(RaisedValues {
                    id: RAISED_VALUES_ID.fetch_add(1, Ordering::SeqCst),
                    values: RefCell::new(Vec::new()),
                });
                module.set_extra_value(v);
                v
            }
        };
        slot.downcast_ref::<RaisedValues<'v>>()
    }
}

/// A raised error value: what `fail(e)` produces.
///
/// It renders as `TypeName: message`, the line the traceback ends on. For a
/// type declared with `traceback = False`, [`RaisedError::exit`] is the
/// `TaskExit` the runtime reports instead, so the error ends the task as
/// `ctx.std.process.exit(1, message)` would. Whoever catches it gets the value
/// back with [`RaisedError::value`].
///
/// Like `TaskExit`, it is only found at the root of an `anyhow` chain, or
/// inside the `EvalError` that wraps it there.
#[derive(Debug)]
pub struct RaisedError {
    /// The store's id and the value's index in it, when it could be parked.
    slot: Option<(u64, usize)>,
    rendered: String,
    exit: Option<TaskExit>,
}

impl RaisedError {
    fn new<'v>(value: Value<'v>, error: ErrorValueRef<'v>, eval: &Evaluator<'v, '_, '_>) -> Self {
        let message = error.message();
        let (rendered, exit) = if error.traceback() {
            (format!("{}: {message}", error.type_name()), None)
        } else {
            (message.to_owned(), Some(TaskExit::error(message)))
        };
        let slot = RaisedValues::of(eval).map(|store| {
            let mut values = store.values.borrow_mut();
            values.push(value);
            (store.id, values.len() - 1)
        });
        Self {
            slot,
            rendered,
            exit,
        }
    }

    /// The exit the raise stands for, if its type has `traceback = False`.
    pub fn exit(&self) -> Option<&TaskExit> {
        self.exit.as_ref()
    }

    /// The raised value, if it was raised in the module `eval` is running.
    pub(super) fn value<'v>(&self, eval: &Evaluator<'v, '_, '_>) -> Option<Value<'v>> {
        let (id, index) = self.slot?;
        let store = eval
            .module()
            .extra_value()?
            .downcast_ref::<RaisedValues<'v>>()?;
        if store.id != id {
            return None;
        }
        store.values.borrow().get(index).copied()
    }

    /// The raised error carried by `err`, however the evaluator wrapped it.
    pub fn from_starlark(err: &starlark::Error) -> Option<&RaisedError> {
        match err.kind() {
            starlark::ErrorKind::Native(e) | starlark::ErrorKind::Other(e) => Self::from_anyhow(e),
            _ => None,
        }
    }

    /// The raised error carried by `err`, following `EvalError` links
    /// however deeply they nest.
    pub fn from_anyhow(err: &anyhow::Error) -> Option<&RaisedError> {
        if let Some(raised) = err.downcast_ref::<RaisedError>() {
            return Some(raised);
        }
        match err.downcast_ref::<EvalError>()? {
            EvalError::StarlarkError(e) => Self::from_starlark(e),
            EvalError::UnknownError(e) => Self::from_anyhow(e),
            _ => None,
        }
    }
}

impl Display for RaisedError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.rendered)
    }
}

impl std::error::Error for RaisedError {}

#[starlark_module]
pub(super) fn register_fail(globals: &mut GlobalsBuilder) {
    /// Fail the execution.
    ///
    /// With a single error value, raises that error itself: its type,
    /// fields, `cause` and `stacktrace` survive, so a caller that catches it
    /// gets back the same value, and one that does not renders it according
    /// to its type's `traceback` setting. Re-raise a caught error the same
    /// way.
    ///
    /// With anything else, behaves as Starlark's `fail`: the arguments are
    /// joined with spaces (strings as-is, other values as their repr) into
    /// the message of a failure that shows its traceback.
    ///
    /// ```starlark
    /// fail(DeployError("no ack", deployment = "prod"))
    /// fail("unexpected state:", state)
    /// ```
    fn fail<'v>(
        #[starlark(args)] args: UnpackTuple<Value<'v>>,
        eval: &mut Evaluator<'v, '_, '_>,
    ) -> starlark::Result<StarlarkNever> {
        if let [value] = args.items[..] {
            if let Some(error) = ErrorValueRef::of(value) {
                let raised = RaisedError::new(value, error, eval);
                return Err(starlark::Error::new_native(anyhow::Error::new(raised)));
            }
        }
        // Starlark's own `fail`, verbatim, so string failures read as before.
        let mut s = String::new();
        for x in args.items {
            s.push(' ');
            match x.unpack_str() {
                Some(x) => s.push_str(x),
                None => x.collect_repr(&mut s),
            }
        }
        Err(starlark::Error::new_kind(starlark::ErrorKind::Fail(
            anyhow::Error::msg(s),
        )))
    }
}
