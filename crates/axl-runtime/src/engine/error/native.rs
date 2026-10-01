//! Error types defined in Rust.
//!
//! [`native_error!`] declares one: a Rust struct with a `message` and its
//! fields, and the AXL error type, derived from `error`, that its values
//! become. A native function raises it by returning it as its error, with `?`
//! like any other; wherever AXL turns the failure into a value (`catch`,
//! `fut.catch()`, a `try_` method's [`Attempt`]) it arrives as an instance of
//! that type, and uncaught it renders as `TypeName: message`.

use std::fmt;
use std::marker::PhantomData;

use starlark::eval::Evaluator;
use starlark::typing::Ty;
use starlark::values::starlark_value_as_type::StarlarkValueAsType;
use starlark::values::string::StarlarkStr;
use starlark::values::type_repr::StarlarkTypeRepr;
use starlark::values::{AllocValue, FrozenValue, Heap, Value};

use super::catch::{caught_ty_with, err_pair, ok_pair};
use super::error_type::NativeErrorType;
use super::value::error_value;
use crate::eval::EvalError;

/// A Rust error that AXL sees as an instance of an error type. Implemented
/// by [`native_error!`].
pub(crate) trait NativeError: fmt::Debug + Send + Sync + 'static {
    /// The error type its values are instances of.
    fn native_type() -> &'static NativeErrorType
    where
        Self: Sized;

    /// [`NativeError::native_type`], for an error whose type is not known.
    fn type_of(&self) -> &'static NativeErrorType;

    fn message(&self) -> &str;

    /// The fields' values, in declaration order.
    fn values<'v>(&self, heap: Heap<'v>) -> Vec<Value<'v>>;
}

/// A type a native error's field can have.
pub(crate) trait NativeFieldType {
    /// The type as a type value, for the field's runtime check.
    fn type_value() -> FrozenValue;
    fn ty() -> Ty;
    fn alloc<'v>(&self, heap: Heap<'v>) -> Value<'v>;
}

impl NativeFieldType for String {
    fn type_value() -> FrozenValue {
        StarlarkValueAsType::<StarlarkStr>::new().to_frozen_value()
    }

    fn ty() -> Ty {
        Ty::string()
    }

    fn alloc<'v>(&self, heap: Heap<'v>) -> Value<'v> {
        heap.alloc_str(self).to_value()
    }
}

/// A native error in flight: what `anyhow::Error` carries for it.
#[derive(Debug)]
pub(crate) struct NativeRaised(Box<dyn NativeError>);

impl NativeRaised {
    pub(crate) fn new(error: impl NativeError) -> Self {
        Self(Box::new(error))
    }

    /// The native error `err` carries, however the evaluator wrapped it.
    pub(super) fn from_anyhow(err: &anyhow::Error) -> Option<&Self> {
        if let Some(raised) = err.downcast_ref::<Self>() {
            return Some(raised);
        }
        match err.downcast_ref::<EvalError>()? {
            EvalError::StarlarkError(e) => Self::from_starlark(e),
            EvalError::UnknownError(e) => Self::from_anyhow(e),
            _ => None,
        }
    }

    pub(super) fn from_starlark(err: &starlark::Error) -> Option<&Self> {
        match err.kind() {
            starlark::ErrorKind::Native(e) | starlark::ErrorKind::Other(e) => Self::from_anyhow(e),
            _ => None,
        }
    }

    /// The error as a value of its type, raised where `eval` is now.
    pub(super) fn to_value<'v>(&self, eval: &Evaluator<'v, '_, '_>) -> Value<'v> {
        let typ = eval
            .frozen_heap()
            .alloc(self.0.type_of().error_type())
            .to_value();
        let values = self.0.values(eval.heap());
        error_value(typ, self.0.message().to_owned(), values, eval)
    }
}

impl fmt::Display for NativeRaised {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}: {}", self.0.type_of().name(), self.0.message())
    }
}

impl std::error::Error for NativeRaised {}

/// What a `try_` method returns: `(None, value)` when the call succeeds,
/// `(err, None)` when it fails with an `E`, typed
/// `tuple[E | None, T | None]`. Any other failure still raises.
pub(crate) struct Attempt<'v, E, T>(Value<'v>, PhantomData<fn() -> (E, T)>);

impl<'v, E: NativeError, T: AllocValue<'v>> Attempt<'v, E, T> {
    pub(crate) fn new(
        result: anyhow::Result<T>,
        eval: &Evaluator<'v, '_, '_>,
    ) -> anyhow::Result<Self> {
        let heap = eval.heap();
        let pair = match result {
            Ok(value) => ok_pair(heap.alloc(value), heap),
            Err(err) => match NativeRaised::from_anyhow(&err) {
                Some(raised) if raised.0.type_of().is_a(E::native_type()) => {
                    err_pair(raised.to_value(eval), heap)
                }
                _ => return Err(err),
            },
        };
        Ok(Self(pair, PhantomData))
    }
}

impl<E: NativeError, T: StarlarkTypeRepr> StarlarkTypeRepr for Attempt<'_, E, T> {
    type Canonical = Self;

    fn starlark_type_repr() -> Ty {
        caught_ty_with(E::native_type().instance_ty(), T::starlark_type_repr())
    }
}

impl<'v, E: NativeError, T: StarlarkTypeRepr> AllocValue<'v> for Attempt<'v, E, T> {
    fn alloc_value(self, _heap: Heap<'v>) -> Value<'v> {
        self.0
    }
}

/// Declare an error type defined in Rust and derived from `error`.
///
/// ```ignore
/// native_error! {
///     /// What the type's documentation page says.
///     pub(crate) struct IoError = "std.io.Error" {
///         /// What the field's documentation says.
///         kind: String,
///     }
/// }
/// ```
///
/// The struct gets a `message: String` and the fields, all public. Return it
/// from a native function as its error, `Err(IoError { .. })?`, and AXL sees a
/// `std.io.Error`; register `IoError::native_type().error_type()` in the
/// globals under its name.
macro_rules! native_error {
    (
        $(#[doc = $doc:literal])*
        $vis:vis struct $name:ident = $type_name:literal {
            $(
                $(#[doc = $field_doc:literal])*
                $field:ident: $field_ty:ty
            ),* $(,)?
        }
    ) => {
        $(#[doc = $doc])*
        #[derive(Debug)]
        $vis struct $name {
            pub message: String,
            $(
                $(#[doc = $field_doc])*
                pub $field: $field_ty,
            )*
        }

        impl $crate::engine::error::NativeError for $name {
            fn native_type() -> &'static $crate::engine::error::NativeErrorType {
                static TYPE: ::std::sync::LazyLock<$crate::engine::error::NativeErrorType> =
                    ::std::sync::LazyLock::new(|| {
                        $crate::engine::error::NativeErrorType::new(
                            $type_name,
                            concat!($($doc, "\n"),*),
                            vec![$(
                                $crate::engine::error::NativeField {
                                    name: stringify!($field),
                                    typ: <$field_ty as $crate::engine::error::NativeFieldType>::type_value(),
                                    ty: <$field_ty as $crate::engine::error::NativeFieldType>::ty(),
                                    docs: concat!($($field_doc, "\n"),*),
                                },
                            )*],
                        )
                    });
                &TYPE
            }

            fn type_of(&self) -> &'static $crate::engine::error::NativeErrorType {
                <Self as $crate::engine::error::NativeError>::native_type()
            }

            fn message(&self) -> &str {
                &self.message
            }

            fn values<'v>(
                &self,
                #[allow(unused)] heap: ::starlark::values::Heap<'v>,
            ) -> Vec<::starlark::values::Value<'v>> {
                vec![$(
                    $crate::engine::error::NativeFieldType::alloc(&self.$field, heap),
                )*]
            }
        }

        impl From<$name> for ::anyhow::Error {
            fn from(error: $name) -> Self {
                ::anyhow::Error::new($crate::engine::error::NativeRaised::new(error))
            }
        }
    };
}

pub(crate) use native_error;
