mod callable;
mod host;
mod instance;
mod memory;
mod types;

use starlark::environment::GlobalsBuilder;
use starlark::values::starlark_value_as_type::StarlarkValueAsType;
use starlark::values::FrozenValue;

use allocative::Allocative;
use anyhow::Context;
use derive_more::Display;

use starlark::environment::Methods;
use starlark::environment::MethodsBuilder;
use starlark::environment::MethodsStatic;
use starlark::starlark_module;
use starlark::starlark_simple_value;
use starlark::values::dict::UnpackDictEntries;
use starlark::values::list::UnpackList;
use starlark::values::NoSerialize;
use starlark::values::ProvidesStaticType;
use starlark::values::Value;
use starlark::values::{self, starlark_value};
use wasmi_wasi::ambient_authority;

use std::cell::RefCell;
use std::rc::Rc;

use host::WasmStoreCtx;

#[starlark_module]
pub fn register_wasm_types(globals: &mut GlobalsBuilder) {
    const Wasm: StarlarkValueAsType<Wasm> = StarlarkValueAsType::new();
    const Callable: StarlarkValueAsType<callable::Callable> = StarlarkValueAsType::new();
    const Exports: StarlarkValueAsType<instance::Exports> = StarlarkValueAsType::new();
    const Instance: StarlarkValueAsType<instance::Instance> = StarlarkValueAsType::new();
    const Memory: StarlarkValueAsType<memory::Memory> = StarlarkValueAsType::new();
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

/// Type alias for host function imports: dict[str, dict[str, callable]]
/// e.g., {"env": {"get_term_size": my_func}}
type HostImports<'v> = UnpackDictEntries<String, UnpackDictEntries<String, Value<'v>>>;

#[starlark_module]
pub(crate) fn wasm_methods(registry: &mut MethodsBuilder) {
    /// Instantiate a WASM module with optional WASI configuration and host function imports.
    ///
    /// # Arguments
    ///
    /// * `path` - Path to the WASM binary file
    /// * `args` - Command-line arguments to pass to the WASM module (WASI)
    /// * `env` - Environment variables to set for the WASM module (WASI)
    /// * `preopened_dirs` - Directories to pre-open for filesystem access (WASI).
    ///   Keys are host paths, values are guest paths.
    /// * `inherit_stdio` - Whether to inherit stdin/stdout/stderr from the host (default: true)
    /// * `imports` - Host function imports, organized by module name and function name.
    ///   **Important:** Host functions must be imported from a loaded module via `load()`,
    ///   not defined in the same file. This ensures they are frozen and can be safely
    ///   stored in the WASM runtime.
    ///
    /// # Returns
    ///
    /// A `wasm.Instance` that can be used to call exported functions.
    ///
    /// # Example
    ///
    /// ```starlark
    /// # In host_funcs.axl:
    /// def get_term_size(ctx, memory) -> tuple[int, int]:
    ///     return (80, 24)
    ///
    /// # In main.axl:
    /// load("./host_funcs.axl", "get_term_size")
    ///
    /// instance = ctx.wasm.instantiate(
    ///     "app.wasm",
    ///     args = ["--verbose"],
    ///     env = {"HOME": "/home/user"},
    ///     preopened_dirs = {"/tmp": "/sandbox"},
    ///     imports = {
    ///         "env": {
    ///             "get_term_size": get_term_size,
    ///         }
    ///     },
    /// )
    /// instance.start()  # Call _start for WASI modules
    /// result = instance.exports.my_function(42)
    /// ```
    ///
    /// # Host Function Signature
    ///
    /// Host functions receive two injected arguments followed by WASM arguments:
    /// - `ctx` - Task context (currently `None`, reserved for future use)
    /// - `memory` - WASM memory access (currently `None`, reserved for future use)
    /// - Additional arguments correspond to the WASM function signature
    ///
    /// Return values are converted back to WASM types. Use tuples for multi-value returns.
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
        #[starlark(require = named, default = UnpackDictEntries::default())] imports: HostImports<
            'v,
        >,
    ) -> anyhow::Result<instance::Instance> {
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

        let wasi_ctx = wasi.build();

        // Check if we have any host function imports
        let has_imports = !imports.entries.is_empty();

        let bytes = std::fs::read(path.as_str()).context(format!(
            "failed to read wasm binary at path {}",
            path.as_str()
        ))?;

        if has_imports {
            // Use host function path with WasmStoreCtx
            instantiate_with_imports(wasi_ctx, &bytes, imports)
        } else {
            // Use simple path (also uses WasmStoreCtx for consistency)
            instantiate_simple(wasi_ctx, &bytes)
        }
    }
}

/// Simple instantiation without host functions.
///
/// Uses WasmStoreCtx for consistency with host function path.
fn instantiate_simple(
    wasi: wasmi_wasi::WasiCtx,
    bytes: &[u8],
) -> anyhow::Result<instance::Instance> {
    let mut config = wasmi::Config::default();
    config.compilation_mode(wasmi::CompilationMode::Eager);
    let engine = wasmi::Engine::new(&config);

    // Create store context (no host functions)
    let store_ctx = WasmStoreCtx::new(wasi);
    let mut store = wasmi::Store::new(&engine, store_ctx);

    let module = wasmi::Module::new(store.engine(), bytes)?;
    let mut linker = wasmi::Linker::new(store.engine());

    wasmi_wasi::add_to_linker(&mut linker, |ctx: &mut WasmStoreCtx| &mut ctx.wasi)?;

    let instance = linker.instantiate_and_start(&mut store, &module)?;

    Ok(instance::Instance {
        module: Rc::new(RefCell::new(module)),
        store: Rc::new(RefCell::new(store)),
        instance: Rc::new(RefCell::new(instance)),
    })
}

/// Require that a value is a frozen function (imported via `load()`).
///
/// Host functions must be defined in a separate file and imported via `load()`.
/// This ensures they are frozen and can be safely stored in `'static` context.
fn require_frozen_function(
    value: Value<'_>,
    module_name: &str,
    func_name: &str,
) -> anyhow::Result<FrozenValue> {
    value.unpack_frozen().ok_or_else(|| {
        anyhow::anyhow!(
            "host function '{}.{}' must be imported from a loaded module (via load()), \
             not defined in the same file as the task.",
            module_name,
            func_name
        )
    })
}

/// Instantiation with host function imports.
///
/// Host functions must be imported from a loaded module (via `load()`).
/// This ensures they are frozen and can be safely stored in `'static` context.
fn instantiate_with_imports<'v>(
    wasi: wasmi_wasi::WasiCtx,
    bytes: &[u8],
    imports: HostImports<'v>,
) -> anyhow::Result<instance::Instance> {
    let mut config = wasmi::Config::default();
    config.compilation_mode(wasmi::CompilationMode::Eager);
    let engine = wasmi::Engine::new(&config);

    // Create store context
    let mut store_ctx = WasmStoreCtx::new(wasi);

    // Collect host function names for linker registration
    let mut func_keys: Vec<(String, String)> = Vec::new();

    // Validate and register host functions
    for (module_name, funcs) in imports.entries {
        for (func_name, func_value) in funcs.entries {
            // Require that the function is frozen (imported via load())
            let frozen = require_frozen_function(func_value, &module_name, &func_name)?;

            // Register in the store context
            store_ctx.register_func(module_name.clone(), func_name.clone(), frozen);
            func_keys.push((module_name.clone(), func_name));
        }
    }

    let mut store = wasmi::Store::new(&engine, store_ctx);

    let module = wasmi::Module::new(store.engine(), bytes)?;
    let mut linker = wasmi::Linker::new(store.engine());

    // Add WASI functions
    wasmi_wasi::add_to_linker(&mut linker, |ctx: &mut WasmStoreCtx| &mut ctx.wasi)?;

    // Register host function callbacks with the linker
    for (module_name, func_name) in func_keys {
        // Get expected signature from WASM module imports
        let expected_sig = host::get_import_signature(&module, &module_name, &func_name);

        if let Some(sig) = expected_sig {
            // Create a safe callback that uses WasmStoreCtx
            let callback = host::create_host_callback(module_name.clone(), func_name.clone());
            linker.func_new(&module_name, &func_name, sig, callback)?;
        }
        // If WASM doesn't import this function, that's OK - just skip
    }

    // Use instantiate() instead of instantiate_and_start() so that _start
    // is not called automatically. The user should call instance.start() manually.
    let pre_instance = linker.instantiate(&mut store, &module)?;
    let instance = pre_instance.ensure_no_start(&mut store).map_err(|e| {
        anyhow::anyhow!("WASM module has start function that must be called: {}", e)
    })?;

    Ok(instance::Instance {
        module: Rc::new(RefCell::new(module)),
        store: Rc::new(RefCell::new(store)),
        instance: Rc::new(RefCell::new(instance)),
    })
}
