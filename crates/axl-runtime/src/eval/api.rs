use crate::engine;
use crate::eval::multi_phase::ModuleEnv;
use starlark::environment::{GlobalsBuilder, LibraryExtension};
use starlark::eval::Evaluator;
use starlark::syntax::{AstModule, Dialect, DialectTypes};

/// Returns a GlobalsBuilder for AXL globals, extending various Starlark library extensions
/// with custom top-level functions registered from the engine module.
pub fn get_globals() -> GlobalsBuilder {
    let mut globals = GlobalsBuilder::extended_by(&[
        LibraryExtension::Breakpoint,
        LibraryExtension::CallStack,
        LibraryExtension::Debug,
        LibraryExtension::EnumType,
        LibraryExtension::Filter,
        // NB: `LibraryExtension::Json` is intentionally skipped — we
        // register a custom `json` namespace below that adds `try_decode`
        // alongside `encode`/`decode`. Calling `globals.namespace("json",
        // ...)` after the stdlib extension would *replace* the namespace
        // rather than merge, so we own it here in full.
        LibraryExtension::Map,
        LibraryExtension::NamespaceType,
        LibraryExtension::Partial,
        LibraryExtension::Pprint,
        LibraryExtension::Prepr,
        LibraryExtension::Print,
        LibraryExtension::Pstr,
        LibraryExtension::RecordType,
        LibraryExtension::SetType,
        LibraryExtension::StructType,
        LibraryExtension::Typing,
    ]);
    engine::builtins::register_json(&mut globals);
    engine::register_globals(&mut globals);
    crate::trace::register_globals(&mut globals);
    globals
}

/// Test harness for evaluating AXL Starlark snippets.
///
/// Evaluates `src` with the full AXL globals and an `AxlLoader` wired to the
/// builtin modules (`@std`, `@aspect`), so `load("@std//hash.axl", ...)` works.
#[cfg(test)]
pub fn eval_expr(src: &str) -> anyhow::Result<String> {
    use std::path::PathBuf;

    use starlark::environment::Module;
    use starlark::eval::Evaluator;
    use starlark::syntax::AstModule;
    use tokio::runtime::Runtime;

    use crate::eval::load::AxlLoader;

    let manifest_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let repo_root = manifest_dir.clone();

    // Env::new calls Handle::current(), which requires a Tokio runtime.
    let rt = Runtime::new()?;
    let _guard = rt.enter();

    // Tests don't exercise dep modules. `@std//` loads work without any
    // caller scope; relative or subpath loads from snippets are unsupported
    // (no `Mod` scope to resolve against).
    let loader = AxlLoader::new("test".to_string(), repo_root.clone(), &[]);

    let ast = AstModule::parse("test", src.to_owned(), &dialect())
        .map_err(|e| anyhow::anyhow!("{}", e))?;

    Module::with_temp_heap(|module| {
        let mut eval = Evaluator::new(&module);
        eval.set_loader(&loader);
        let val = eval
            .eval_module(ast, &loader.globals)
            .map_err(|e| anyhow::anyhow!("{}", e))?;
        Ok(val.to_repr())
    })
}

/// Evaluate an AXL code snippet with the full set of AXL globals.
///
/// Useful in tests and tooling that need to evaluate inline Starlark without
/// touching the filesystem. Returns `Ok(())` if evaluation succeeds, or a
/// `starlark::Error` describing the failure.
pub fn eval_snippet(code: &str) -> starlark::Result<()> {
    use crate::engine::store::Env;
    use std::path::PathBuf;
    use tokio::runtime::Runtime;

    let ast = AstModule::parse("<snippet>", code.to_owned(), &dialect())?;
    let globals = get_globals().build();
    // `feature()` and `task()` read Env from `eval.extra`. Env::new requires
    // a tokio runtime (it captures the current Handle).
    let rt = Runtime::new().map_err(|e| anyhow::anyhow!("failed to create runtime: {}", e))?;
    let _guard = rt.enter();
    let env_store = Env::new("test".to_string(), PathBuf::from("/"));
    ModuleEnv::with(|env| {
        let mut eval = Evaluator::new(&env.0);
        eval.extra = Some(&env_store);
        eval.eval_module(ast, &globals).map(|_| ())
    })
}

pub fn dialect() -> Dialect {
    Dialect {
        enable_def: true,
        enable_lambda: true,
        enable_load: true,
        enable_load_reexport: true,
        enable_keyword_only_arguments: true,
        enable_positional_only_arguments: true,
        enable_types: DialectTypes::Enable,
        enable_f_strings: true,
        enable_top_level_stmt: true,
        ..Default::default()
    }
}
