#![allow(clippy::new_without_default)]
pub mod builtins;
pub mod docs;
pub mod engine;
pub mod eval;
pub mod module;
pub mod trace;

/// Bazel subprocess live-tracking. Re-exported so `aspect-cli`'s
/// signal handler can forward SIGINT / SIGTERM to in-flight bazel
/// clients on shutdown without exposing the rest of the bazel
/// engine internals.
pub mod bazel_live {
    pub use crate::engine::bazel::live::*;
}

#[cfg(test)]
pub mod test;

#[cfg(test)]
#[macro_export]
macro_rules! axl_eval {
    ($code:expr $(,)?) => {
        $crate::test::eval($code).with_loader().repr()
    };
}

#[cfg(test)]
#[macro_export]
macro_rules! axl_check {
    ($code:expr $(,)?) => {
        $crate::test::eval($code).check()
    };
}
