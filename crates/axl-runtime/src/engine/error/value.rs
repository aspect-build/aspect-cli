//! Error values and the attributes every one of them has.

use std::fmt::{self, Display, Write};

use allocative::Allocative;
use starlark::environment::{Methods, MethodsBuilder, MethodsStatic};
use starlark::eval::CallStack;
use starlark::starlark_module;
use starlark::values::list::AllocList;
use starlark::values::{
    Freeze, FrozenValue, Heap, NoSerialize, ProvidesStaticType, StarlarkValue, Trace, Value,
    ValueLifetimeless, ValueLike, starlark_value,
};

use super::error_type::{ErrorTypeMeta, ErrorTypeRef, ROOT_META};
use super::frame::ErrorFrame;

pub(super) static ERROR_METHODS: MethodsStatic = MethodsStatic::new("error_methods", error_methods);

/// An error value. `typ` is the error type it was built from, or `None` for a
/// plain `error` the runtime built from a Rust error.
#[derive(Debug, Trace, Freeze, ProvidesStaticType, NoSerialize, Allocative)]
pub struct ErrorValueGen<V: ValueLifetimeless> {
    pub(super) typ: V,
    #[trace(static)]
    pub(super) message: String,
    pub(super) cause: V,
    pub(super) values: Box<[V]>,
    #[trace(static)]
    #[freeze(identity)]
    #[allocative(skip)]
    pub(super) stack: CallStack,
}

pub type ErrorValue<'v> = ErrorValueGen<Value<'v>>;
pub type FrozenErrorValue = ErrorValueGen<FrozenValue>;

impl<'v, V: ValueLike<'v>> Display for ErrorValueGen<V> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.message)
    }
}

/// A live or frozen error value, read without caring which.
#[derive(Clone, Copy)]
pub(crate) enum ErrorValueRef<'v> {
    Live(&'v ErrorValue<'v>),
    Frozen(&'v FrozenErrorValue),
}

impl<'v> ErrorValueRef<'v> {
    pub(crate) fn of(value: Value<'v>) -> Option<Self> {
        if let Some(e) = value.downcast_ref::<ErrorValue<'v>>() {
            Some(Self::Live(e))
        } else {
            value.downcast_ref::<FrozenErrorValue>().map(Self::Frozen)
        }
    }

    fn typ(self) -> Option<ErrorTypeRef<'v>> {
        let typ = match self {
            Self::Live(e) => e.typ,
            Self::Frozen(e) => e.typ.to_value(),
        };
        ErrorTypeRef::of(typ)
    }

    fn meta(self) -> &'v ErrorTypeMeta {
        self.typ().map_or(&ROOT_META, |t| t.meta())
    }

    pub(super) fn type_name(self) -> &'v str {
        self.typ().map_or("error", |t| t.name())
    }

    pub(super) fn message(self) -> &'v str {
        match self {
            Self::Live(e) => &e.message,
            Self::Frozen(e) => &e.message,
        }
    }

    fn cause(self) -> Value<'v> {
        match self {
            Self::Live(e) => e.cause,
            Self::Frozen(e) => e.cause.to_value(),
        }
    }

    fn stack(self) -> &'v CallStack {
        match self {
            Self::Live(e) => &e.stack,
            Self::Frozen(e) => &e.stack,
        }
    }

    /// Whether this error is an instance of the error type with id `type_id`.
    pub(crate) fn is_instance_of(self, type_id: u64) -> bool {
        self.meta().ancestors.contains(&type_id)
    }

    pub(super) fn traceback(self) -> bool {
        self.meta().traceback
    }
}

fn this_error<'v>(this: Value<'v>) -> ErrorValueRef<'v> {
    ErrorValueRef::of(this).expect("error attribute bound on a non-error value")
}

#[starlark_value(type = "error")]
impl<'v, V: ValueLike<'v>> StarlarkValue<'v> for ErrorValueGen<V>
where
    Self: ProvidesStaticType<'v>,
{
    type Canonical = FrozenErrorValue;

    fn collect_repr(&self, collector: &mut String) {
        let name = ErrorTypeRef::of(self.typ.to_value()).map_or("error", |t| t.name());
        write!(collector, "{name}(message = {:?}", self.message).unwrap();
        if let Some(t) = ErrorTypeRef::of(self.typ.to_value()) {
            for (i, v) in self.values.iter().enumerate() {
                write!(collector, ", {} = ", t.field(i).0).unwrap();
                v.to_value().collect_repr(collector);
            }
        }
        if !self.cause.to_value().is_none() {
            collector.push_str(", cause = ");
            self.cause.to_value().collect_repr(collector);
        }
        collector.push(')');
    }

    fn collect_str(&self, collector: &mut String) {
        collector.push_str(&self.message);
    }

    fn get_attr(&self, attribute: &str, _heap: Heap<'v>) -> Option<Value<'v>> {
        let t = ErrorTypeRef::of(self.typ.to_value())?;
        t.field_index(attribute).map(|i| self.values[i].to_value())
    }

    fn has_attr(&self, attribute: &str, _heap: Heap<'v>) -> bool {
        ErrorTypeRef::of(self.typ.to_value()).is_some_and(|t| t.field_index(attribute).is_some())
    }

    // `message`, `cause` and `stacktrace` are attributes, listed already.
    fn dir_attr(&self) -> Vec<String> {
        ErrorTypeRef::of(self.typ.to_value()).map_or_else(Vec::new, |t| {
            (0..t.field_count())
                .map(|i| t.field(i).0.to_owned())
                .collect()
        })
    }

    fn get_methods() -> Option<&'static Methods> {
        Some(ERROR_METHODS.methods())
    }
}

#[starlark_module]
fn error_methods(builder: &mut MethodsBuilder) {
    /// What went wrong, as given to the constructor. `str(e)` returns it too.
    #[starlark(attribute)]
    fn message<'v>(this: Value<'v>) -> starlark::Result<String> {
        Ok(this_error(this).message().to_owned())
    }

    /// The error that led to this one, or `None`.
    #[starlark(attribute)]
    fn cause<'v>(this: Value<'v>) -> starlark::Result<Value<'v>> {
        Ok(this_error(this).cause())
    }

    /// Where the error was constructed, as a list of frames, outermost call
    /// first. For an error the runtime built from a failed operation, it is
    /// where that failure reached AXL, e.g. the `future.block()` call.
    #[starlark(attribute)]
    fn stacktrace<'v>(this: Value<'v>, heap: Heap<'v>) -> starlark::Result<Value<'v>> {
        // A frame records where its function was *called*, so, like
        // Starlark's traceback, pair each location with the caller's name.
        // The innermost frame is the error's own constructor, and drops out.
        let mut caller = "<module>";
        let mut frames = Vec::new();
        for frame in &this_error(this).stack().frames {
            frames.push(ErrorFrame::new(caller, frame.location.as_ref()));
            caller = &frame.name;
        }
        Ok(heap.alloc(AllocList(frames)))
    }
}
