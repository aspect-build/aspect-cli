#![allow(clippy::new_without_default)]
pub mod builtins;
pub mod docs;
pub mod engine;
pub mod eval;
pub mod module;
pub mod trace;

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
