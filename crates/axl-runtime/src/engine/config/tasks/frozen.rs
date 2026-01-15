//! Helper functions for freezing Starlark values.

use anyhow::anyhow;
use starlark::environment::Module;
use starlark::values::OwnedFrozenValue;
use starlark::values::Value;

use crate::eval::EvalError;

/// Helper to freeze a Value using a temporary module.
///
/// This encapsulates the unsafe transmute pattern used throughout the codebase.
/// The returned OwnedFrozenValue keeps the temporary heap alive.
pub fn freeze_value(value: Value<'_>) -> Result<OwnedFrozenValue, EvalError> {
    let temp_module = Module::new();
    // SAFETY: We're moving the value into a temporary module that we control.
    // The transmute is needed because the type system can't prove the value
    // will outlive the module, but we know it will because we freeze immediately.
    let short_value: Value = unsafe { std::mem::transmute(value) };
    temp_module.set("__temp__", short_value);
    let frozen = temp_module
        .freeze()
        .map_err(|e| EvalError::UnknownError(anyhow!("{:?}", e)))?;
    frozen
        .get("__temp__")
        .map_err(|e: anyhow::Error| EvalError::UnknownError(e))
}
