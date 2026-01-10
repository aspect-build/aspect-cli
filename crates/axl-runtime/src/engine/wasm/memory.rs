use allocative::Allocative;
use anyhow::Context;
use derive_more::Display;

use starlark::environment::Methods;
use starlark::environment::MethodsBuilder;
use starlark::environment::MethodsStatic;
use starlark::starlark_module;
use starlark::values::none::NoneType;
use starlark::values::AllocValue;
use starlark::values::Heap;
use starlark::values::NoSerialize;
use starlark::values::ProvidesStaticType;
use starlark::values::Value;
use starlark::values::ValueLike;
use starlark::values::{self, starlark_value};
use starlark::StarlarkResultExt;
use starlark_derive::Trace;
use wasmi::AsContext;
use wasmi::AsContextMut;

use std::cell::RefCell;
use std::fmt::Debug;
use std::rc::Rc;

use super::host::WasmStoreCtx;
use crate::engine::types::bytes::Bytes;

#[derive(Display, Trace, ProvidesStaticType, NoSerialize, Allocative)]
#[display("<wasm.Memory>")]
pub struct Memory {
    #[allocative(skip)]
    pub(crate) store: Rc<RefCell<wasmi::Store<WasmStoreCtx>>>,
    #[allocative(skip)]
    pub(crate) memory: Rc<RefCell<wasmi::Memory>>,
}

impl Debug for Memory {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("wasm.Memory")
            .field("memory", &self.memory)
            .finish()
    }
}

#[starlark_value(type = "Memory")]
impl<'v> values::StarlarkValue<'v> for Memory {
    fn get_methods() -> Option<&'static Methods> {
        static RES: MethodsStatic = MethodsStatic::new();
        RES.methods(memory_methods)
    }
}

#[starlark_module]
pub(crate) fn memory_methods(registry: &mut MethodsBuilder) {
    fn grow<'v>(
        this: values::Value<'v>,
        #[starlark(require = pos)] by: u64,
    ) -> anyhow::Result<u64> {
        let wm = this.downcast_ref_err::<Memory>()?;
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
        let wm = this.downcast_ref_err::<Memory>()?;
        let mem = wm.memory.borrow();
        let iter = buffer
            .iterate(heap)
            .into_anyhow_result()
            .context("buffer is not iterable")?;
        let bytes: Vec<u8> = iter
            .map(|x| {
                x.unpack_i32()
                    .ok_or_else(|| anyhow::anyhow!("buffer element is not an integer"))
                    .map(|v| v as u8)
            })
            .collect::<anyhow::Result<Vec<u8>>>()?;
        mem.write(wm.store.borrow_mut().as_context_mut(), offset, &bytes)?;
        Ok(NoneType)
    }

    /// Reads bytes from wasm linear memory.
    ///
    /// # Arguments
    /// * `offset` - The byte offset in wasm memory to start reading from
    /// * `length` - The number of bytes to read
    ///
    /// # Returns
    /// A `Bytes` object containing the data read from memory.
    ///
    /// # Example
    /// ```starlark
    /// # Get pointer and length from a wasm function
    /// result = wasm_instance.exports.get_data()
    /// ptr = result & 0xFFFFFFFF
    /// length = (result >> 32) & 0xFFFFFFFF
    ///
    /// # Read the data from wasm memory
    /// data = memory.read(ptr, length)
    /// print(str(data))
    /// ```
    fn read<'v>(
        this: values::Value<'v>,
        #[starlark(require = pos)] offset: usize,
        #[starlark(require = pos)] length: usize,
    ) -> anyhow::Result<Bytes> {
        let wm = this.downcast_ref_err::<Memory>()?;
        let mem = wm.memory.borrow();
        let mut buffer = vec![0u8; length];
        mem.read(wm.store.borrow_mut().as_context_mut(), offset, &mut buffer)?;
        Ok(Bytes::from(buffer.as_slice()))
    }

    /// Read a UTF-8 string from memory with explicit length.
    ///
    /// # Arguments
    /// * `ptr` - The byte offset in wasm memory to start reading from
    /// * `len` - The number of bytes to read
    ///
    /// # Returns
    /// The decoded UTF-8 string.
    fn read_string<'v>(
        this: values::Value<'v>,
        #[starlark(require = pos)] ptr: usize,
        #[starlark(require = pos)] len: usize,
    ) -> anyhow::Result<String> {
        let wm = this.downcast_ref_err::<Memory>()?;
        let mem = wm.memory.borrow();
        let mut buffer = vec![0u8; len];
        mem.read(wm.store.borrow_mut().as_context_mut(), ptr, &mut buffer)?;
        String::from_utf8(buffer)
            .map_err(|e| anyhow::anyhow!("invalid UTF-8 at ptr {}: {}", ptr, e))
    }

    /// Read a null-terminated C string from memory.
    ///
    /// # Arguments
    /// * `ptr` - The byte offset in wasm memory to start reading from
    /// * `max_len` - Maximum number of bytes to scan for null terminator (default: 4096)
    ///
    /// # Returns
    /// The decoded UTF-8 string (without the null terminator).
    fn read_cstring<'v>(
        this: values::Value<'v>,
        #[starlark(require = pos)] ptr: usize,
        #[starlark(require = named, default = 4096)] max_len: usize,
    ) -> anyhow::Result<String> {
        let wm = this.downcast_ref_err::<Memory>()?;
        let mem = wm.memory.borrow();
        let store = wm.store.borrow();
        let data = mem.data(store.as_context());

        // Find null terminator
        let end = data[ptr..]
            .iter()
            .take(max_len)
            .position(|&b| b == 0)
            .ok_or_else(|| anyhow::anyhow!("no null terminator found within {} bytes", max_len))?;

        String::from_utf8(data[ptr..ptr + end].to_vec())
            .map_err(|e| anyhow::anyhow!("invalid UTF-8: {}", e))
    }

    /// Write a string to memory at the given pointer.
    ///
    /// # Arguments
    /// * `ptr` - The byte offset in wasm memory to write to
    /// * `s` - The string to write
    ///
    /// # Returns
    /// The number of bytes written.
    fn write_string<'v>(
        this: values::Value<'v>,
        #[starlark(require = pos)] ptr: usize,
        #[starlark(require = pos)] s: &str,
    ) -> anyhow::Result<usize> {
        let wm = this.downcast_ref_err::<Memory>()?;
        let mem = wm.memory.borrow();
        let bytes = s.as_bytes();
        mem.write(wm.store.borrow_mut().as_context_mut(), ptr, bytes)?;
        Ok(bytes.len())
    }
}

impl<'v> AllocValue<'v> for Memory {
    fn alloc_value(self, heap: &'v Heap) -> values::Value<'v> {
        heap.alloc_complex_no_freeze(self)
    }
}
