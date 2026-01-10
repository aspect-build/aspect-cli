use allocative::Allocative;
use derive_more::Display;

use starlark::starlark_simple_value;
use starlark::values::NoSerialize;
use starlark::values::ProvidesStaticType;
use starlark::values::{self, starlark_value};

/// Marker type for WASM i32 type annotation in Starlark.
///
/// Used in host function signatures to indicate a 32-bit integer parameter/return.
///
/// # Example
/// ```starlark
/// def get_value(ctx_mem: tuple[TaskContext, wasm.Memory]) -> wasm.i32:
///     return 42
/// ```
#[derive(Debug, Display, ProvidesStaticType, NoSerialize, Allocative)]
#[display("wasm.i32")]
pub struct WasmI32;

#[starlark_value(type = "wasm.i32")]
impl<'v> values::StarlarkValue<'v> for WasmI32 {}

starlark_simple_value!(WasmI32);

/// Marker type for WASM i64 type annotation in Starlark.
///
/// Used in host function signatures to indicate a 64-bit integer parameter/return.
///
/// # Example
/// ```starlark
/// def get_large_value(ctx_mem: tuple[TaskContext, wasm.Memory]) -> wasm.i64:
///     return 9223372036854775807
/// ```
#[derive(Debug, Display, ProvidesStaticType, NoSerialize, Allocative)]
#[display("wasm.i64")]
pub struct WasmI64;

#[starlark_value(type = "wasm.i64")]
impl<'v> values::StarlarkValue<'v> for WasmI64 {}

starlark_simple_value!(WasmI64);

/// Marker type for WASM f32 type annotation in Starlark.
///
/// Used in host function signatures to indicate a 32-bit float parameter/return.
///
/// # Example
/// ```starlark
/// def get_float(ctx_mem: tuple[TaskContext, wasm.Memory]) -> wasm.f32:
///     return 3.14
/// ```
#[derive(Debug, Display, ProvidesStaticType, NoSerialize, Allocative)]
#[display("wasm.f32")]
pub struct WasmF32;

#[starlark_value(type = "wasm.f32")]
impl<'v> values::StarlarkValue<'v> for WasmF32 {}

starlark_simple_value!(WasmF32);

/// Marker type for WASM f64 type annotation in Starlark.
///
/// Used in host function signatures to indicate a 64-bit float parameter/return.
///
/// # Example
/// ```starlark
/// def get_double(ctx_mem: tuple[TaskContext, wasm.Memory]) -> wasm.f64:
///     return 3.14159265358979
/// ```
#[derive(Debug, Display, ProvidesStaticType, NoSerialize, Allocative)]
#[display("wasm.f64")]
pub struct WasmF64;

#[starlark_value(type = "wasm.f64")]
impl<'v> values::StarlarkValue<'v> for WasmF64 {}

starlark_simple_value!(WasmF64);
