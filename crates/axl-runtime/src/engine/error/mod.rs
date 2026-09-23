//! First-class error values: the `error` global, the types derived from it,
//! and the payload that carries a raised error through the evaluator.
//!
//! `error` is itself an error type. Calling any error type builds an error
//! value; `.type(...)` on any error type derives a child type. Every value
//! reports `type(e) == "error"`, and `isinstance(e, T)` holds for `T` and each
//! of its ancestors: an instance's type carries the ids of its whole chain, so
//! matching is a scan of that short list.
//!
//! `fail(e)` raises a value as a [`RaisedError`], the `'static` payload an
//! `anyhow::Error` can hold, while the value itself stays live on the heap.
//! Whoever catches it (today, `future.catch()`) gets back that same value. A
//! type declared with `traceback = False` makes its raised errors end the task
//! the way `ctx.std.process.exit` does: [`RaisedError::exit`] is what the
//! `TaskExit` consumers look for.
//!
//! - `error_type`: the types, `.type(...)`, and the constructor.
//! - `value`: error values and their `message`, `cause`, `stacktrace`.
//! - `frame`: the frames of a `stacktrace`.
//! - `raise`: `fail(e)` and the payload that carries a raised value.
//! - `catch`: turning a failure back into a value, for `future.catch()`.

use starlark::environment::GlobalsBuilder;

mod catch;
mod error_type;
mod frame;
mod raise;
mod value;

#[cfg(test)]
mod test_future;
#[cfg(test)]
mod tests;

pub(crate) use catch::{callback_error, error_value_of};
pub(crate) use error_type::error_type_id;
pub use raise::RaisedError;
pub(crate) use value::ErrorValueRef;

/// Register `error` and the `fail` that raises error values. Call after the
/// Starlark standard library, so this `fail` replaces the builtin one.
pub fn register_globals(globals: &mut GlobalsBuilder) {
    globals.set("error", error_type::root_error_type());
    raise::register_fail(globals);
    #[cfg(test)]
    test_future::register(globals);
}
