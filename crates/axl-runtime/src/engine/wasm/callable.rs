use allocative::Allocative;
use anyhow::Context;
use derive_more::Display;

use starlark::eval::Arguments;
use starlark::eval::Evaluator;
use starlark::values::float::UnpackFloat;
use starlark::values::tuple::AllocTuple;
use starlark::values::tuple::UnpackTuple;
use starlark::values::AllocValue;
use starlark::values::Heap;
use starlark::values::NoSerialize;
use starlark::values::ProvidesStaticType;
use starlark::values::UnpackValue;
use starlark::values::Value;
use starlark::values::{self, starlark_value};
use starlark_derive::Trace;
use wasmi::AsContext;
use wasmi::AsContextMut;
use wasmi::ResumableCall;

use std::cell::RefCell;
use std::fmt::Debug;
use std::rc::Rc;

use super::host::{starlark_to_wasm_results, wasm_vals_to_starlark, WasmStoreCtx};
use super::memory::Memory;

/// A callable WASM function that can be invoked from Starlark.
///
/// This wraps a WASM function export and handles argument conversion
/// between Starlark and WASM types.
#[derive(Display, Trace, ProvidesStaticType, NoSerialize, Allocative)]
#[display("<wasm.Callable>")]
pub struct Callable {
    pub(crate) name: String,
    #[allocative(skip)]
    pub(crate) store: Rc<RefCell<wasmi::Store<WasmStoreCtx>>>,
    #[allocative(skip)]
    pub(crate) instance: Rc<RefCell<wasmi::Instance>>,
}

impl Debug for Callable {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("wasm.Callable")
            .field("name", &self.name)
            .field("instance", &self.instance)
            .finish()
    }
}

#[starlark_value(type = "Callable")]
impl<'v> values::StarlarkValue<'v> for Callable {
    fn invoke(
        &self,
        _me: Value<'v>,
        args: &Arguments<'v, '_>,
        eval: &mut Evaluator<'v, '_, '_>,
    ) -> starlark::Result<Value<'v>> {
        let mut store = self.store.borrow_mut();
        let instance = self.instance.borrow_mut();
        let func = instance
            .get_func(store.as_context_mut(), &self.name)
            .context(format!("module does not export function `{}`", &self.name))?;
        let ty = func.ty(store.as_context_mut());

        let mut inputs: Vec<wasmi::Val> = vec![];
        let mut outputs: Vec<wasmi::Val> = vec![];
        let heap = eval.heap();

        let positionals = if !ty.params().is_empty() {
            use starlark::__derive_refs::{
                parse_args::{check_required, parse_signature},
                sig::parameter_spec,
            };
            let __args: [_; 1] = parse_signature(
                &parameter_spec("args", &[], &[], true, &[], false),
                args,
                eval.heap(),
            )?;

            let positionals: UnpackTuple<Value<'v>> = check_required("args", __args[0])?;

            positionals.items
        } else {
            vec![]
        };

        if positionals.len() != ty.params().len() {
            return Err(starlark::Error::new_kind(starlark::ErrorKind::Function(
                anyhow::anyhow!(
                    "expected {} arguments, got {}",
                    ty.params().len(),
                    positionals.len(),
                ),
            )));
        }

        for (idx, param) in ty.params().iter().enumerate() {
            let arg = positionals[idx];
            match param {
                wasmi::ValType::I32 => inputs
                    .push(wasmi::Val::I32(arg.unpack_i32().ok_or(anyhow::anyhow!(
                        "argument {idx} should be an integer"
                    ))?)),
                wasmi::ValType::I64 => {
                    // Try to get as i64 first, fall back to i32
                    let val = if let Some(v) = arg.unpack_i32() {
                        v as i64
                    } else {
                        return Err(starlark::Error::new_kind(starlark::ErrorKind::Function(
                            anyhow::anyhow!("argument {idx} should be an integer"),
                        )));
                    };
                    inputs.push(wasmi::Val::I64(val));
                }
                wasmi::ValType::F64 => {
                    let val = UnpackFloat::unpack_value(arg)
                        .map_err(|e| {
                            starlark::Error::new_kind(starlark::ErrorKind::Function(
                                anyhow::anyhow!("argument {idx}: {}", e),
                            ))
                        })?
                        .ok_or_else(|| {
                            starlark::Error::new_kind(starlark::ErrorKind::Function(
                                anyhow::anyhow!("argument {idx} should be a number"),
                            ))
                        })?;
                    inputs.push(wasmi::Val::F64(wasmi::F64::from_float(val.0)));
                }
                wasmi::ValType::F32 => {
                    let val = UnpackFloat::unpack_value(arg)
                        .map_err(|e| {
                            starlark::Error::new_kind(starlark::ErrorKind::Function(
                                anyhow::anyhow!("argument {idx}: {}", e),
                            ))
                        })?
                        .ok_or_else(|| {
                            starlark::Error::new_kind(starlark::ErrorKind::Function(
                                anyhow::anyhow!("argument {idx} should be a number"),
                            ))
                        })?;
                    inputs.push(wasmi::Val::F32(wasmi::F32::from_float(val.0 as f32)));
                }
                wasmi::ValType::V128 => {
                    return Err(starlark::Error::new_kind(starlark::ErrorKind::Function(
                        anyhow::anyhow!("128-bit values are not supported"),
                    )));
                }
                wasmi::ValType::FuncRef => {
                    return Err(starlark::Error::new_kind(starlark::ErrorKind::Function(
                        anyhow::anyhow!("function references are not supported"),
                    )));
                }
                wasmi::ValType::ExternRef => {
                    return Err(starlark::Error::new_kind(starlark::ErrorKind::Function(
                        anyhow::anyhow!("external references are not supported"),
                    )));
                }
            }
        }

        for param in ty.results().iter() {
            match param {
                wasmi::ValType::I32 => outputs.push(wasmi::Val::I32(0)),
                wasmi::ValType::I64 => outputs.push(wasmi::Val::I64(0)),
                wasmi::ValType::F32 => outputs.push(wasmi::Val::F32(wasmi::F32::from_float(0.0))),
                wasmi::ValType::F64 => outputs.push(wasmi::Val::F64(wasmi::F64::from_float(0.0))),
                wasmi::ValType::V128 => {
                    return Err(starlark::Error::new_kind(starlark::ErrorKind::Function(
                        anyhow::anyhow!("128-bit values are not supported"),
                    )));
                }
                wasmi::ValType::FuncRef => {
                    return Err(starlark::Error::new_kind(starlark::ErrorKind::Function(
                        anyhow::anyhow!("function references are not supported"),
                    )));
                }
                wasmi::ValType::ExternRef => {
                    return Err(starlark::Error::new_kind(starlark::ErrorKind::Function(
                        anyhow::anyhow!("external references are not supported"),
                    )));
                }
            }
        }

        // Execute the WASM function using resumable calls for trampoline pattern
        let mut resumable = func
            .call_resumable(store.as_context_mut(), &inputs, &mut outputs)
            .map_err(|e| {
                starlark::Error::new_kind(starlark::ErrorKind::Function(anyhow::anyhow!(
                    "WASM call failed: {}",
                    e
                )))
            })?;

        // Drop borrows before entering the loop (needed for process_pending_host_call)
        drop(instance);
        drop(store);

        loop {
            match resumable {
                ResumableCall::Finished => break,
                ResumableCall::HostTrap(trap) => {
                    // Process the pending host call
                    let host_results =
                        process_pending_host_call(&self.store, &self.instance, eval)?;

                    // Resume execution with the host call results
                    let mut store = self.store.borrow_mut();
                    resumable = trap
                        .resume(store.as_context_mut(), &host_results, &mut outputs)
                        .map_err(|e| {
                            starlark::Error::new_kind(starlark::ErrorKind::Function(
                                anyhow::anyhow!("WASM resume failed: {}", e),
                            ))
                        })?;
                    drop(store);
                }
                ResumableCall::OutOfFuel(_) => {
                    return Err(starlark::Error::new_kind(starlark::ErrorKind::Function(
                        anyhow::anyhow!("WASM execution ran out of fuel"),
                    )));
                }
            }
        }

        let wrap_val = |x: wasmi::Val| match x {
            wasmi::Val::I32(v) => heap.alloc(v),
            wasmi::Val::I64(v) => heap.alloc(v),
            wasmi::Val::F32(v) => heap.alloc(v.to_float() as f64),
            wasmi::Val::F64(v) => heap.alloc(v.to_float()),
            wasmi::Val::V128(_v) => heap.alloc(0), // Unreachable due to earlier check
            wasmi::Val::FuncRef(_v) => heap.alloc(0), // Unreachable due to earlier check
            wasmi::Val::ExternRef(_v) => heap.alloc(0), // Unreachable due to earlier check
        };

        if outputs.is_empty() {
            return Ok(Value::new_none());
        } else if outputs.len() == 1 {
            return Ok(wrap_val(outputs.pop().unwrap()));
        }

        Ok(heap.alloc(AllocTuple(outputs.into_iter().map(wrap_val))))
    }
}

impl<'v> AllocValue<'v> for Callable {
    fn alloc_value(self, heap: &'v Heap) -> values::Value<'v> {
        heap.alloc_complex_no_freeze(self)
    }
}

fn process_pending_host_call<'v>(
    store_rc: &Rc<RefCell<wasmi::Store<WasmStoreCtx>>>,
    instance_rc: &Rc<RefCell<wasmi::Instance>>,
    eval: &mut Evaluator<'v, '_, '_>,
) -> Result<Vec<wasmi::Val>, starlark::Error> {
    let mut store = store_rc.borrow_mut();
    let instance = instance_rc.borrow();

    let pending = store.data_mut().pending_call.take().ok_or_else(|| {
        starlark::Error::new_kind(starlark::ErrorKind::Function(anyhow::anyhow!(
            "No pending host call"
        )))
    })?;

    let func = store
        .data()
        .host_funcs
        .get(&(pending.module_name.clone(), pending.func_name.clone()))
        .copied()
        .ok_or_else(|| {
            starlark::Error::new_kind(starlark::ErrorKind::Function(anyhow::anyhow!(
                "Host function not found: {}.{}",
                pending.module_name,
                pending.func_name
            )))
        })?;

    let heap = eval.heap();

    // Create Memory with owned Rc
    let memory_val = match instance.get_memory(store.as_context(), "memory") {
        Some(mem) => heap.alloc(Memory {
            store: Rc::clone(store_rc),
            memory: Rc::new(RefCell::new(mem)),
        }),
        None => Value::new_none(),
    };

    // Must drop borrows before calling eval_function
    drop(instance);
    drop(store);

    // Args: [ctx, memory, ...wasm_args]
    // ctx is None for now (reserved for future TaskContext support)
    let ctx_val = Value::new_none();
    let mut starlark_args = vec![ctx_val, memory_val];
    starlark_args.extend(wasm_vals_to_starlark(&pending.args, heap));

    let result = eval.eval_function(func.to_value(), &starlark_args, &[])?;

    let mut wasm_results = pending.expected_results.clone();
    starlark_to_wasm_results(result, &mut wasm_results, heap).map_err(|e| {
        starlark::Error::new_kind(starlark::ErrorKind::Function(anyhow::anyhow!("{}", e)))
    })?;

    Ok(wasm_results)
}
