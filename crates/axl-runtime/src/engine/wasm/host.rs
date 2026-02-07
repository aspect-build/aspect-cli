//! Host function bridging for WASM modules.
//!
//! This module provides the infrastructure for calling Starlark functions from WASM code
//! using a trampoline pattern that allows host functions to access WASM memory.
//!
//! # Architecture (Trampoline Pattern)
//!
//! Host functions are stored as `FrozenValue` (which is `Copy + 'static`) in `WasmStoreCtx`.
//! When WASM calls a host function:
//!
//! 1. The callback records the pending call in `WasmStoreCtx.pending_call` and traps
//! 2. Control returns to `Callable::invoke` which has ownership of the Store
//! 3. The pending call is processed with full access to Memory and Evaluator
//! 4. WASM execution resumes with the return values via `ResumableCall`
//!
//! This design is completely safe - no `unsafe` code is needed because:
//! - `FrozenValue` is `'static` and `Copy`
//! - The trampoline returns ownership to Callable::invoke before calling Starlark
//! - Memory access works because we have owned `Rc<RefCell<Store>>`

use std::collections::HashMap;

use starlark::values::FrozenValue;
use starlark::values::Heap;
use starlark::values::UnpackValue;
use starlark::values::Value;
use starlark::values::float::UnpackFloat;
use wasmi::Caller;
use wasmi_wasi::WasiCtx;

/// Marker for the host call trap used by the trampoline.
pub const HOST_CALL_TRAP_MARKER: &str = "__HOST_CALL_PENDING__";

/// A pending host function call waiting to be handled by the trampoline.
#[derive(Clone, Debug)]
pub struct PendingHostCall {
    pub module_name: String,
    pub func_name: String,
    pub args: Vec<wasmi::Val>,
    pub expected_results: Vec<wasmi::Val>,
}

/// Store context for WASM instances with host functions.
///
/// All fields are `'static` - no lifetime parameters needed.
/// This is the data type used for `wasmi::Store<WasmStoreCtx>`.
///
/// Note: The frozen context is a `FrozenValue` (not `OwnedFrozenValue`) because
/// the heap is kept alive by the `OwnedFrozenValue` in `execute_task_with_args`.
pub struct WasmStoreCtx {
    /// WASI context for system calls
    pub wasi: WasiCtx,
    /// Host functions indexed by (module, name)
    pub host_funcs: HashMap<(String, String), FrozenValue>,
    /// Frozen TaskContext for host function access
    pub frozen_context: Option<FrozenValue>,
    /// Pending host function call for trampoline pattern
    pub pending_call: Option<PendingHostCall>,
}

impl WasmStoreCtx {
    /// Create a new WasmStoreCtx with the given WASI context.
    pub fn new(wasi: WasiCtx) -> Self {
        Self {
            wasi,
            host_funcs: HashMap::new(),
            frozen_context: None,
            pending_call: None,
        }
    }

    /// Set the frozen TaskContext.
    /// The heap is kept alive by OwnedFrozenValue in execute_task_with_args.
    pub fn set_frozen_context(&mut self, context: Option<FrozenValue>) {
        self.frozen_context = context;
    }

    /// Get the frozen TaskContext value (if set).
    pub fn frozen_context(&self) -> Option<FrozenValue> {
        self.frozen_context
    }

    /// Register a host function.
    pub fn register_func(&mut self, module: String, name: String, func: FrozenValue) {
        self.host_funcs.insert((module, name), func);
    }
}

/// Get the expected import signature from a WASM module.
pub fn get_import_signature(
    module: &wasmi::Module,
    module_name: &str,
    func_name: &str,
) -> Option<wasmi::FuncType> {
    for import in module.imports() {
        if import.module() == module_name && import.name() == func_name {
            if let wasmi::ExternType::Func(func_type) = import.ty() {
                return Some(func_type.clone());
            }
        }
    }
    None
}

/// Convert WASM values to Starlark values.
pub fn wasm_vals_to_starlark<'v>(vals: &[wasmi::Val], heap: &'v Heap) -> Vec<Value<'v>> {
    vals.iter()
        .map(|val| match val {
            wasmi::Val::I32(v) => heap.alloc(*v),
            wasmi::Val::I64(v) => heap.alloc(*v),
            wasmi::Val::F32(v) => heap.alloc(v.to_float() as f64),
            wasmi::Val::F64(v) => heap.alloc(v.to_float()),
            wasmi::Val::V128(_) => heap.alloc(0),
            wasmi::Val::FuncRef(_) => heap.alloc(0),
            wasmi::Val::ExternRef(_) => heap.alloc(0),
        })
        .collect()
}

/// Convert a single Starlark value to a WASM value.
pub fn convert_single_value(val: Value<'_>, result: &mut wasmi::Val) -> Result<(), wasmi::Error> {
    match result {
        wasmi::Val::I32(_) => {
            if let Some(v) = val.unpack_i32() {
                *result = wasmi::Val::I32(v);
            } else {
                return Err(wasmi::Error::new(
                    "host function returned non-integer for i32 result",
                ));
            }
        }
        wasmi::Val::I64(_) => {
            if let Some(v) = val.unpack_i32() {
                *result = wasmi::Val::I64(v as i64);
            } else {
                return Err(wasmi::Error::new(
                    "host function returned non-integer for i64 result",
                ));
            }
        }
        wasmi::Val::F32(_) => {
            if let Ok(Some(v)) = UnpackFloat::unpack_value(val) {
                *result = wasmi::Val::F32(wasmi::core::F32::from_float(v.0 as f32));
            } else {
                return Err(wasmi::Error::new(
                    "host function returned non-number for f32 result",
                ));
            }
        }
        wasmi::Val::F64(_) => {
            if let Ok(Some(v)) = UnpackFloat::unpack_value(val) {
                *result = wasmi::Val::F64(wasmi::core::F64::from_float(v.0));
            } else {
                return Err(wasmi::Error::new(
                    "host function returned non-number for f64 result",
                ));
            }
        }
        _ => {
            return Err(wasmi::Error::new(
                "unsupported WASM return type (v128, funcref, externref)",
            ));
        }
    }
    Ok(())
}

/// Convert Starlark return value(s) to WASM values.
///
/// Handles single values, tuples (multi-return), and None.
pub fn starlark_to_wasm_results<'v>(
    val: Value<'v>,
    results: &mut [wasmi::Val],
    heap: &'v Heap,
) -> Result<(), wasmi::Error> {
    if val.is_none() {
        return Ok(());
    }

    // Check if it's a tuple by looking at the type
    let type_name = val.get_type();
    if type_name == "tuple" {
        // It's a tuple - get length and iterate
        let len = val
            .length()
            .map_err(|e| wasmi::Error::new(format!("failed to get tuple length: {}", e)))?;
        if len as usize != results.len() {
            return Err(wasmi::Error::new(format!(
                "host function returned {} values, expected {}",
                len,
                results.len()
            )));
        }
        for i in 0..len as usize {
            // Create index value using heap
            let index = heap.alloc(i as i32);
            let item = val
                .at(index, heap)
                .map_err(|e| wasmi::Error::new(format!("failed to get tuple item {}: {}", i, e)))?;
            convert_single_value(item, &mut results[i])?;
        }
        return Ok(());
    }

    // Single value
    if results.len() == 1 {
        convert_single_value(val, &mut results[0])
    } else if results.is_empty() {
        // Function returned a value but WASM expects void - that's OK
        Ok(())
    } else {
        Err(wasmi::Error::new(format!(
            "host function returned 1 value, expected {}",
            results.len()
        )))
    }
}

pub fn create_host_callback(
    module_name: String,
    func_name: String,
) -> impl Fn(Caller<'_, WasmStoreCtx>, &[wasmi::Val], &mut [wasmi::Val]) -> Result<(), wasmi::Error>
+ Send
+ Sync
+ 'static {
    move |mut caller, args, results| {
        let store_ctx = caller.data_mut();

        // Record the pending call
        store_ctx.pending_call = Some(PendingHostCall {
            module_name: module_name.clone(),
            func_name: func_name.clone(),
            args: args.to_vec(),
            expected_results: results.iter().cloned().collect(),
        });

        // Trap to return control to Callable::invoke
        Err(wasmi::Error::new(HOST_CALL_TRAP_MARKER))
    }
}
