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
