//! Typed handles for the JDK types generated code and its consumers touch.
//!
//! A handle owns one local reference, is `!Send`, and takes generics as
//! phantom parameters. Java generics are erased at run time, so an element
//! type such as `List<'a, String<'a>>` is a promise the caller makes, not
//! something the JVM checks; reading an element that is not that type is a
//! `ClassCastException` at the first call on it. Where a Rust trait fits, a
//! handle implements it: `Iterable`, `Collection`, `List` and `Set` are
//! `IntoIterator<Item = Result<T>>`, `Iterator` is an `Iterator`, `List` is
//! `FromIterator` and `Extend`, `String` converts to and from Rust strings,
//! and the boxed numbers and `String` are `Ord`.
//!
//! Statics and constructors take the `JNIEnv` they make references in;
//! instance methods attach their thread themselves.

pub mod io;
pub mod lang;
pub mod math;
pub mod time;
pub mod util;
