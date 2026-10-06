#![allow(clippy::new_without_default)]

// Self-alias so the `service_server` proc-macro (which emits
// `::axl_runtime::...` paths) resolves when invoked from this crate.
extern crate self as axl_runtime;

pub mod banner;
pub mod builtins;
pub mod ci;
pub mod color;
pub mod diag;
pub mod docs;
pub mod engine;
pub mod eval;
pub mod module;
pub mod out;
pub mod project_root;
pub mod trace;

pub use eval::TaskExit;

pub use engine::cancellation::{Kind as SignalKind, Signals};

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
