use crate::engine;
use starlark::environment::Globals;
use starlark::environment::GlobalsBuilder;
use starlark::environment::LibraryExtension;
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

/// Returns the globals surface for `*_test.axl` files: the full AXL surface
/// plus the test-only vocabulary (`assert`, …). The extra names exist *only*
/// in test files — the loader selects this surface by filename suffix (see
/// `eval::load`) — so test scaffolding can never leak into production AXL.
pub fn get_test_globals() -> Globals {
    let mut globals = get_globals();
    engine::testing::register_test_globals(&mut globals);
    globals.build()
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
