use allocative::Allocative;
use anyhow::Context;
use derive_more::Display;

use starlark::environment::Methods;
use starlark::environment::MethodsBuilder;
use starlark::environment::MethodsStatic;
use starlark::eval::Arguments;
use starlark::eval::Evaluator;
use starlark::values::dict::UnpackDictEntries;
use starlark::values::list::UnpackList;
use starlark::values::none::NoneType;
use starlark::values::tuple::AllocTuple;
use starlark::values::tuple::UnpackTuple;
use starlark::values::AllocValue;
use starlark::values::Heap;
use starlark::values::Value;
use starlark::values::ValueLike;
use starlark::StarlarkResultExt;
use starlark_derive::Trace;
use wasmi_wasi::ambient_authority;

use std::cell::RefCell;
use std::fmt::Debug;
use std::rc::Rc;
use wasmi::AsContext;
use wasmi::AsContextMut;
use wasmi_wasi::WasiCtx;

use starlark::starlark_module;
use starlark::starlark_simple_value;
use starlark::values;
use starlark::values::starlark_value;
use starlark::values::NoSerialize;
use starlark::values::ProvidesStaticType;

#[derive(Display, Trace, ProvidesStaticType, NoSerialize, Allocative)]
#[display("<wasm.WasmMemory>")]
pub struct WasmMemory {
    #[allocative(skip)]
    store: Rc<RefCell<wasmi::Store<WasiCtx>>>,
    #[allocative(skip)]
    memory: Rc<RefCell<wasmi::Memory>>,
}

impl Debug for WasmMemory {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("wasm.WasmMemory")
            .field("memory", &self.memory)
            .finish()
    }
}

#[starlark_value(type = "wasm.WasmMemory")]
impl<'v> values::StarlarkValue<'v> for WasmMemory {
    fn get_methods() -> Option<&'static Methods> {
        static RES: MethodsStatic = MethodsStatic::new();
        RES.methods(wasm_memory_methods)
    }
}

#[starlark_module]
pub(crate) fn wasm_memory_methods(registry: &mut MethodsBuilder) {
    fn grow<'v>(
        this: values::Value<'v>,
        #[starlark(require = pos)] by: u64,
    ) -> anyhow::Result<u64> {
        let wm = this.downcast_ref::<WasmMemory>().unwrap();
        let mem = wm.memory.borrow();
        let pages = mem.grow(wm.store.borrow_mut().as_context_mut(), by)?;
        Ok(pages)
    }
    fn write<'v>(
        this: values::Value<'v>,
        #[starlark(require = pos)] offset: usize,
        #[starlark(require = pos)] buffer: Value<'v>,
        heap: &'v values::Heap,
    ) -> anyhow::Result<NoneType> {
        let wm = this.downcast_ref::<WasmMemory>().unwrap();
        let mem = wm.memory.borrow();
        let iter = buffer
            .iterate(heap)
            .into_anyhow_result()
            .context("buffer is not iterable")?;
        mem.write(
            wm.store.borrow_mut().as_context_mut(),
            offset,
            iter.map(|x| x.unpack_i32().expect("invalid integer") as u8)
                .collect::<Vec<u8>>()
                .as_slice(),
        )?;
        Ok(NoneType)
    }
}

impl<'v> AllocValue<'v> for WasmMemory {
    fn alloc_value(self, heap: &'v values::Heap) -> values::Value<'v> {
        heap.alloc_complex_no_freeze(self)
    }
}

#[derive(Display, Trace, ProvidesStaticType, NoSerialize, Allocative)]
#[display("<wasm.WasmCallable>")]
pub struct WasmCallable {
    name: String,
    #[allocative(skip)]
    store: Rc<RefCell<wasmi::Store<WasiCtx>>>,
    #[allocative(skip)]
    instance: Rc<RefCell<wasmi::Instance>>,
}

impl Debug for WasmCallable {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("wasm.WasmCallable")
            .field("instance", &self.instance)
            .finish()
    }
}

#[starlark_value(type = "wasm.WasmCallable")]
impl<'v> values::StarlarkValue<'v> for WasmCallable {
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
                wasmi::ValType::I64 => inputs.push(wasmi::Val::I64(
                    arg.unpack_i32()
                        .ok_or(anyhow::anyhow!("argument {idx} should be an integer"))?
                        as i64,
                )),
                wasmi::ValType::F64 => inputs.push(wasmi::Val::F64(wasmi::F64::from_float(
                    arg.unpack_i32()
                        .ok_or(anyhow::anyhow!("argument {idx} should be a float"))?
                        as f64,
                ))),
                wasmi::ValType::F32 => inputs.push(wasmi::Val::F32(wasmi::F32::from_float(
                    arg.unpack_i32()
                        .ok_or(anyhow::anyhow!("argument {idx} should be a float"))?
                        as f32,
                ))),
                wasmi::ValType::V128 => todo!("Implement support for 128-bit values"),
                wasmi::ValType::FuncRef => todo!("Implement support for function references"),
                wasmi::ValType::ExternRef => todo!("Implement support for external references"),
            }
        }

        for param in ty.results().iter() {
            match param {
                wasmi::ValType::I32 => outputs.push(wasmi::Val::I32(0)),
                wasmi::ValType::I64 => outputs.push(wasmi::Val::I64(0)),
                wasmi::ValType::F32 => outputs.push(wasmi::Val::F32(wasmi::F32::from_float(0.0))),
                wasmi::ValType::F64 => outputs.push(wasmi::Val::F64(wasmi::F64::from_float(0.0))),
                wasmi::ValType::V128 => todo!("Implement support for 128-bit values"),
                wasmi::ValType::FuncRef => todo!("Implement support for function references"),
                wasmi::ValType::ExternRef => todo!("Implement support for external references"),
            }
        }

        func.call(store.as_context_mut(), &inputs, &mut outputs)
            .expect("failed to call");

        let wrap_val = |x: wasmi::Val| match x {
            wasmi::Val::I32(v) => heap.alloc(v),
            wasmi::Val::I64(v) => heap.alloc(v),
            wasmi::Val::F32(v) => heap.alloc(v.to_float() as f64),
            wasmi::Val::F64(v) => heap.alloc(v.to_float()),
            wasmi::Val::V128(_v) => todo!("Implement support for 128-bit values"),
            wasmi::Val::FuncRef(_v) => todo!("Implement support for function references"),
            wasmi::Val::ExternRef(_v) => todo!("Implement support for external references"),
        };

        if outputs.is_empty() {
            return Ok(Value::new_none());
        } else if outputs.len() == 1 {
            return Ok(wrap_val(outputs.pop().unwrap()));
        }

        Ok(heap.alloc(AllocTuple(outputs.into_iter().map(wrap_val))))
    }
}

impl<'v> AllocValue<'v> for WasmCallable {
    fn alloc_value(self, heap: &'v values::Heap) -> values::Value<'v> {
        heap.alloc_complex_no_freeze(self)
    }
}

#[derive(Display, Trace, ProvidesStaticType, NoSerialize, Allocative)]
#[display("<warm.WasmExports>")]
pub struct WasmExports {
    #[allocative(skip)]
    module: Rc<RefCell<wasmi::Module>>,
    #[allocative(skip)]
    store: Rc<RefCell<wasmi::Store<WasiCtx>>>,
    #[allocative(skip)]
    instance: Rc<RefCell<wasmi::Instance>>,
}

impl Debug for WasmExports {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("wasm.WasmExports")
            .field("module", &self.module)
            .field("instance", &self.instance)
            .finish()
    }
}

#[starlark_value(type = "wasm.WasmExports")]
impl<'v> values::StarlarkValue<'v> for WasmExports {
    fn get_attr(&self, _attribute: &str, heap: &'v Heap) -> Option<Value<'v>> {
        Some(heap.alloc(WasmCallable {
            name: _attribute.to_string(),
            instance: Rc::clone(&self.instance),
            store: Rc::clone(&self.store),
        }))
    }
}

impl<'v> AllocValue<'v> for WasmExports {
    fn alloc_value(self, heap: &'v values::Heap) -> values::Value<'v> {
        heap.alloc_complex_no_freeze(self)
    }
}

#[derive(Display, Trace, ProvidesStaticType, NoSerialize, Allocative)]
#[display("<wasm.WasmInstance>")]
pub struct WasmInstance {
    #[allocative(skip)]
    module: Rc<RefCell<wasmi::Module>>,
    #[allocative(skip)]
    store: Rc<RefCell<wasmi::Store<WasiCtx>>>,
    #[allocative(skip)]
    instance: Rc<RefCell<wasmi::Instance>>,
}

impl Debug for WasmInstance {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("wasm.WasmInstance")
            .field("module", &self.module)
            .field("instance", &self.instance)
            .finish()
    }
}

#[starlark_value(type = "wasm.WasmInstance")]
impl<'v> values::StarlarkValue<'v> for WasmInstance {
    fn get_methods() -> Option<&'static Methods> {
        static RES: MethodsStatic = MethodsStatic::new();
        RES.methods(wasm_instance_methods)
    }
}

impl<'v> AllocValue<'v> for WasmInstance {
    fn alloc_value(self, heap: &'v values::Heap) -> values::Value<'v> {
        heap.alloc_complex_no_freeze(self)
    }
}

#[starlark_module]
pub(crate) fn wasm_instance_methods(registry: &mut MethodsBuilder) {
    fn get_memory<'v>(
        this: values::Value<'v>,
        #[starlark(require = pos)] name: values::StringValue,
    ) -> anyhow::Result<WasmMemory> {
        let wi = this.downcast_ref::<WasmInstance>().unwrap();
        let mut store = wi.store.borrow_mut();
        let memory = wi
            .instance
            .borrow()
            .get_memory(store.as_context_mut(), name.as_str())
            .unwrap();
        Ok(WasmMemory {
            store: Rc::clone(&wi.store),
            memory: Rc::new(RefCell::new(memory)),
        })
    }

    #[starlark(attribute)]
    fn exports<'v>(this: values::Value<'v>) -> anyhow::Result<WasmExports> {
        let wi = this.downcast_ref::<WasmInstance>().unwrap();
        Ok(WasmExports {
            module: Rc::clone(&wi.module),
            store: Rc::clone(&wi.store),
            instance: Rc::clone(&wi.instance),
        })
    }

    fn start<'v>(this: values::Value<'v>) -> anyhow::Result<bool> {
        let wi = this.downcast_ref::<WasmInstance>().unwrap();
        let mut store = wi.store.borrow_mut();
        let instance = wi.instance.borrow_mut();
        let func = instance.get_func(store.as_context(), "_start").unwrap();
        let inputs: Vec<wasmi::Val> = vec![];
        let mut outputs: Vec<wasmi::Val> = vec![];
        let _ = func.call(store.as_context_mut(), &inputs, &mut outputs);

        Ok(true)
    }
}

#[derive(Debug, Display, ProvidesStaticType, NoSerialize, Allocative)]
#[display("<wasm.Wasm>")]
pub struct Wasm {}

impl Wasm {
    pub fn new() -> Self {
        Self {}
    }
}

#[starlark_value(type = "wasm.Wasm")]
impl<'v> values::StarlarkValue<'v> for Wasm {
    fn get_methods() -> Option<&'static Methods> {
        static RES: MethodsStatic = MethodsStatic::new();
        RES.methods(wasm_methods)
    }
}

starlark_simple_value!(Wasm);

#[starlark_module]
pub(crate) fn wasm_methods(registry: &mut MethodsBuilder) {
    fn instantiate<'v>(
        #[allow(unused)] this: values::Value<'v>,
        #[starlark(require = pos)] path: values::StringValue,
        #[starlark(require = named, default = UnpackList::default())] args: UnpackList<String>,
        #[starlark(require = named, default = UnpackDictEntries::default())] env: UnpackDictEntries<
            String,
            values::StringValue,
        >,
        #[starlark(require = named, default = UnpackDictEntries::default())]
        preopened_dirs: UnpackDictEntries<values::StringValue, values::StringValue>,
        #[starlark(require = named, default = true)] inherit_stdio: bool,
    ) -> anyhow::Result<WasmInstance> {
        let mut wasi = wasmi_wasi::WasiCtxBuilder::new();

        if inherit_stdio {
            wasi.inherit_stdio();
        }

        for arg in args.items {
            wasi.arg(arg.as_str())?;
        }

        for (key, value) in env.entries {
            wasi.env(key.as_str(), value.as_str())?;
        }

        for (key, value) in preopened_dirs.entries {
            wasi.preopened_dir(
                wasmi_wasi::Dir::open_ambient_dir(key.as_str(), ambient_authority())?,
                value.as_str(),
            )?;
        }

        let wasi = wasi.build();
        let mut config = wasmi::Config::default();
        config.compilation_mode(wasmi::CompilationMode::Eager);
        let mut store = wasmi::Store::new(&wasmi::Engine::new(&config), wasi);
        let bytes = std::fs::read(path.as_str())
            .context("failed to read wasm binary at path {path.as_str()}")?;
        let module = wasmi::Module::new(store.engine(), bytes)?;
        let mut linker = <wasmi::Linker<WasiCtx>>::new(store.engine());

        wasmi_wasi::add_to_linker(&mut linker, |ctx| ctx)?;

        let instance = linker.instantiate_and_start(&mut store, &module)?;

        Ok(WasmInstance {
            module: Rc::new(RefCell::new(module)),
            store: Rc::new(RefCell::new(store)),
            instance: Rc::new(RefCell::new(instance)),
        })
    }
}
