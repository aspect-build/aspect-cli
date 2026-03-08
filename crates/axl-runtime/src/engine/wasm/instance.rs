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

use std::fmt::Debug;
use std::sync::Arc;
use std::sync::Mutex;

use super::callable::Callable;
use super::host::WasmStoreCtx;
use super::memory::Memory;

#[derive(Display, Trace, ProvidesStaticType, NoSerialize, Allocative)]
#[display("<wasm.WasmExports>")]
pub struct Exports {
    #[allocative(skip)]
    #[allow(dead_code)]
    pub(crate) module: Arc<Mutex<wasmi::Module>>,
    #[allocative(skip)]
    pub(crate) store: Arc<Mutex<wasmi::Store<WasmStoreCtx>>>,
    #[allocative(skip)]
    pub(crate) instance: Arc<Mutex<wasmi::Instance>>,
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
    fn get_attr(&self, attribute: &str, heap: Heap<'v>) -> Option<Value<'v>> {
        Some(heap.alloc(Callable {
            name: attribute.to_string(),
            instance: Arc::clone(&self.instance),
            store: Arc::clone(&self.store),
        }))
    }
}

impl<'v> AllocValue<'v> for Exports {
    fn alloc_value(self, heap: Heap<'v>) -> values::Value<'v> {
        heap.alloc_complex_no_freeze(self)
    }
}

#[derive(Display, Trace, ProvidesStaticType, NoSerialize, Allocative)]
#[display("<wasm.Instance>")]
pub struct Instance {
    #[allocative(skip)]
    pub(crate) module: Arc<Mutex<wasmi::Module>>,
    #[allocative(skip)]
    pub(crate) store: Arc<Mutex<wasmi::Store<WasmStoreCtx>>>,
    #[allocative(skip)]
    pub(crate) instance: Arc<Mutex<wasmi::Instance>>,
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
    fn alloc_value(self, heap: Heap<'v>) -> values::Value<'v> {
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
        let mut store = wi.store.lock().unwrap();
        let memory = wi
            .instance
            .lock()
            .unwrap()
            .get_memory(store.as_context_mut(), name.as_str())
            .ok_or_else(|| {
                anyhow::anyhow!("memory '{}' not found in WASM module", name.as_str())
            })?;
        Ok(Memory {
            store: Arc::clone(&wi.store),
            memory: Arc::new(Mutex::new(memory)),
        })
    }

    #[starlark(attribute)]
    fn exports<'v>(this: values::Value<'v>) -> anyhow::Result<Exports> {
        let wi = this
            .downcast_ref::<Instance>()
            .ok_or_else(|| anyhow::anyhow!("expected WasmInstance"))?;
        Ok(Exports {
            module: Arc::clone(&wi.module),
            store: Arc::clone(&wi.store),
            instance: Arc::clone(&wi.instance),
        })
    }

    fn start<'v>(this: values::Value<'v>) -> anyhow::Result<bool> {
        let wi = this
            .downcast_ref::<Instance>()
            .ok_or_else(|| anyhow::anyhow!("expected WasmInstance"))?;
        let mut store = wi.store.lock().unwrap();
        let instance = wi.instance.lock().unwrap();
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
        let store = wi.store.lock().unwrap();
        Ok(wi
            .instance
            .lock()
            .unwrap()
            .exports(store.as_context())
            .map(|e| e.name().to_string())
            .collect())
    }
}
