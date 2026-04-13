use crate::engine;
use starlark::environment::{GlobalsBuilder, LibraryExtension};
use starlark::syntax::{Dialect, DialectTypes};

/// Returns a GlobalsBuilder for AXL globals, extending various Starlark library extensions
/// with custom top-level functions registered from the engine module.
pub fn get_globals() -> GlobalsBuilder {
    let mut globals = GlobalsBuilder::extended_by(&[
        LibraryExtension::Breakpoint,
        LibraryExtension::CallStack,
        LibraryExtension::Debug,
        LibraryExtension::EnumType,
        LibraryExtension::Filter,
        LibraryExtension::Json,
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
    engine::register_globals(&mut globals);
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

    use crate::eval::load::{AxlLoader, ModuleScope};

    let manifest_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let deps_root = manifest_dir.join("../aspect-cli/src/builtins");
    let repo_root = manifest_dir.clone();

    // AxlStore::new calls Handle::current(), which requires a Tokio runtime.
    let rt = Runtime::new()?;
    let _guard = rt.enter();

    let loader = AxlLoader::new("test".to_string(), repo_root.clone(), deps_root);

    // Seed the stacks so that load() can resolve the parent path and module scope.
    loader.module_stack.borrow_mut().push(ModuleScope {
        name: "test".to_string(),
        path: repo_root.clone(),
    });
    loader
        .load_stack
        .borrow_mut()
        .push(repo_root.join("test.axl"));

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
