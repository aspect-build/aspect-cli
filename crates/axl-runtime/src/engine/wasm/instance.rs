use allocative::Allocative;
use derive_more::Display;

use starlark::environment::Methods;
use starlark::environment::MethodsBuilder;
use starlark::environment::MethodsStatic;
use starlark::starlark_module;
use starlark::values::AllocValue;
use starlark::values::Heap;
use starlark::values::NoSerialize;
use starlark::values::ProvidesStaticType;
use starlark::values::Value;
use starlark::values::ValueLike;
use starlark::values::{self, starlark_value};
use starlark_derive::Trace;
use wasmi::AsContext;
use wasmi::AsContextMut;

use std::cell::RefCell;
use std::fmt::Debug;
use std::rc::Rc;

use super::callable::Callable;
use super::host::WasmStoreCtx;
use super::memory::Memory;

#[derive(Display, Trace, ProvidesStaticType, NoSerialize, Allocative)]
#[display("<wasm.WasmExports>")]
pub struct Exports {
    #[allocative(skip)]
    #[allow(dead_code)]
    pub(crate) module: Rc<RefCell<wasmi::Module>>,
    #[allocative(skip)]
    pub(crate) store: Rc<RefCell<wasmi::Store<WasmStoreCtx>>>,
    #[allocative(skip)]
    pub(crate) instance: Rc<RefCell<wasmi::Instance>>,
}

impl Debug for Exports {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("wasm.Exports")
            .field("module", &self.module)
            .field("instance", &self.instance)
            .finish()
    }
}

#[starlark_value(type = "Exports")]
impl<'v> values::StarlarkValue<'v> for Exports {
    fn get_attr(&self, attribute: &str, heap: &'v Heap) -> Option<Value<'v>> {
        Some(heap.alloc(Callable {
            name: attribute.to_string(),
            instance: Rc::clone(&self.instance),
            store: Rc::clone(&self.store),
        }))
    }
}

impl<'v> AllocValue<'v> for Exports {
    fn alloc_value(self, heap: &'v Heap) -> values::Value<'v> {
        heap.alloc_complex_no_freeze(self)
    }
}

#[derive(Display, Trace, ProvidesStaticType, NoSerialize, Allocative)]
#[display("<wasm.Instance>")]
pub struct Instance {
    #[allocative(skip)]
    pub(crate) module: Rc<RefCell<wasmi::Module>>,
    #[allocative(skip)]
    pub(crate) store: Rc<RefCell<wasmi::Store<WasmStoreCtx>>>,
    #[allocative(skip)]
    pub(crate) instance: Rc<RefCell<wasmi::Instance>>,
}

impl Debug for Instance {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("wasm.Instance")
            .field("module", &self.module)
            .field("instance", &self.instance)
            .finish()
    }
}

#[starlark_value(type = "Instance")]
impl<'v> values::StarlarkValue<'v> for Instance {
    fn get_methods() -> Option<&'static Methods> {
        static RES: MethodsStatic = MethodsStatic::new();
        RES.methods(instance_methods)
    }
}

impl<'v> AllocValue<'v> for Instance {
    fn alloc_value(self, heap: &'v Heap) -> values::Value<'v> {
        heap.alloc_complex_no_freeze(self)
    }
}

#[starlark_module]
pub(crate) fn instance_methods(registry: &mut MethodsBuilder) {
    fn get_memory<'v>(
        this: values::Value<'v>,
        #[starlark(require = pos)] name: values::StringValue,
    ) -> anyhow::Result<Memory> {
        let wi = this
            .downcast_ref::<Instance>()
            .ok_or_else(|| anyhow::anyhow!("expected Instance"))?;
        let mut store = wi.store.borrow_mut();
        let memory = wi
            .instance
            .borrow()
            .get_memory(store.as_context_mut(), name.as_str())
            .ok_or_else(|| {
                anyhow::anyhow!("memory '{}' not found in WASM module", name.as_str())
            })?;
        Ok(Memory {
            store: Rc::clone(&wi.store),
            memory: Rc::new(RefCell::new(memory)),
        })
    }

    #[starlark(attribute)]
    fn exports<'v>(this: values::Value<'v>) -> anyhow::Result<Exports> {
        let wi = this
            .downcast_ref::<Instance>()
            .ok_or_else(|| anyhow::anyhow!("expected WasmInstance"))?;
        Ok(Exports {
            module: Rc::clone(&wi.module),
            store: Rc::clone(&wi.store),
            instance: Rc::clone(&wi.instance),
        })
    }

    fn start<'v>(this: values::Value<'v>) -> anyhow::Result<bool> {
        let wi = this
            .downcast_ref::<Instance>()
            .ok_or_else(|| anyhow::anyhow!("expected WasmInstance"))?;
        let mut store = wi.store.borrow_mut();
        let instance = wi.instance.borrow_mut();
        let func = instance
            .get_func(store.as_context(), "_start")
            .ok_or_else(|| anyhow::anyhow!("WASM module does not export '_start' function"))?;
        let inputs: Vec<wasmi::Val> = vec![];
        let mut outputs: Vec<wasmi::Val> = vec![];
        match func.call(store.as_context_mut(), &inputs, &mut outputs) {
            Ok(()) => Ok(true),
            Err(e) => {
                // Go wasip1 modules call proc_exit(0) on success, which wasmi reports as an error.
                // Treat exit code 0 as success.
                let err_str = e.to_string();
                if err_str.contains("exit status 0") || err_str.contains("ExitCode(0)") {
                    Ok(true)
                } else {
                    Err(anyhow::anyhow!("_start failed: {}", e))
                }
            }
        }
    }

    /// Returns a list of all exported names from the WASM module.
    fn list_exports<'v>(this: values::Value<'v>) -> anyhow::Result<Vec<String>> {
        let wi = this
            .downcast_ref::<Instance>()
            .ok_or_else(|| anyhow::anyhow!("expected WasmInstance"))?;
        let store = wi.store.borrow();
        Ok(wi
            .instance
            .borrow()
            .exports(store.as_context())
            .map(|e| e.name().to_string())
            .collect())
    }
}
