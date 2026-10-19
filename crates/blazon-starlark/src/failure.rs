//! What a script sees when reading an attribute fails.
//!
//! Starlark's attribute hook returns an `Option`, so it cannot raise: `None`
//! is "no such attribute". A Java exception or a bridge error while reading
//! an attribute that does exist is returned as an [`AttributeFailure`]
//! instead, a value that raises the original message on every use that can
//! raise, and prints as the message.

use crate::error;
use allocative::Allocative;
use blazon_rt::Error;
use starlark::collections::StarlarkHasher;
use starlark::starlark_simple_value;
use starlark::values::{
    Heap, NoSerialize, ProvidesStaticType, StarlarkValue, Value, starlark_value,
};
use std::fmt;

#[derive(ProvidesStaticType, NoSerialize, Allocative, Debug)]
pub struct AttributeFailure {
    message: String,
}

starlark_simple_value!(AttributeFailure);

impl AttributeFailure {
    /// The error as it was reported: the exception line and its first frame.
    pub fn message(&self) -> &str {
        &self.message
    }

    fn raise<T>(&self) -> starlark::Result<T> {
        Err(error(Error::msg(self.message.clone())))
    }
}

/// The value standing for a failed read of attribute `name`.
pub fn failed<'v>(name: &str, e: &Error, heap: Heap<'v>) -> Value<'v> {
    heap.alloc(AttributeFailure {
        message: format!("reading attribute `{name}` failed: {e}"),
    })
}

impl fmt::Display for AttributeFailure {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "<error: {}>", self.message)
    }
}

#[starlark_value(type = "error")]
impl<'v> StarlarkValue<'v> for AttributeFailure {
    fn equals(&self, _other: Value<'v>) -> starlark::Result<bool> {
        self.raise()
    }

    fn write_hash(&self, _hasher: &mut StarlarkHasher) -> starlark::Result<()> {
        self.raise()
    }

    fn compare(&self, _other: Value<'v>) -> starlark::Result<std::cmp::Ordering> {
        self.raise()
    }

    fn length(&self) -> starlark::Result<i32> {
        self.raise()
    }

    fn at(&self, _index: Value<'v>, _heap: Heap<'v>) -> starlark::Result<Value<'v>> {
        self.raise()
    }

    fn is_in(&self, _other: Value<'v>) -> starlark::Result<bool> {
        self.raise()
    }

    fn add(&self, _other: Value<'v>, _heap: Heap<'v>) -> Option<starlark::Result<Value<'v>>> {
        Some(self.raise())
    }

    fn iterate_collect(&self, _heap: Heap<'v>) -> starlark::Result<Vec<Value<'v>>> {
        self.raise()
    }

    fn invoke(
        &self,
        _me: Value<'v>,
        _args: &starlark::eval::Arguments<'v, '_>,
        _eval: &mut starlark::eval::Evaluator<'v, '_, '_>,
    ) -> starlark::Result<Value<'v>> {
        self.raise()
    }
}
