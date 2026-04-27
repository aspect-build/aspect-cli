//! Documentation extraction for tooling (e.g. `axl-docgen`).
//!
//! Bundles documentation for both the Rust-defined globals and the embedded
//! `@std//*.axl` builtin modules.

use crate::builtins::{self, STD_DIR};
use crate::eval::Loader;
use crate::eval::api::get_globals;
use anyhow::anyhow;
use starlark::docs::DocModule;
use std::path::PathBuf;

/// All AXL documentation in one bundle: Rust-defined types/globals plus the
/// per-`@std//` builtin modules.
pub struct Documentation {
    /// Documentation for all Rust-defined types and globals (the result of
    /// `get_globals().build().documentation()`).
    pub types: DocModule,
    /// Documentation for each embedded `@std//*.axl` builtin, keyed by the
    /// basename without the `.axl` suffix (e.g. `"time"`).
    pub builtins: Vec<(String, DocModule)>,
}

/// Collect documentation for the entire AXL runtime — both Rust-defined types
/// and the embedded `@std//*.axl` builtin modules.
///
/// Must be called within an active Tokio runtime (the loader's `AxlStore`
/// requires `tokio::runtime::Handle::current()`).
pub fn documentation() -> anyhow::Result<Documentation> {
    let types = get_globals().build().documentation();

    // One loader, reused across every `@std//*.axl` evaluation. `eval_std_module`
    // self-manages the load stack and never reads the module stack (std loads
    // short-circuit before any caller-context lookups in `FileLoader::load`),
    // so no seeding is required.
    let cwd = std::env::current_dir().unwrap_or_else(|_| PathBuf::from("/"));
    let loader = Loader::new("docgen".to_string(), cwd.clone(), cwd);

    let mut builtins = Vec::new();
    for filename in list_std_files() {
        let content = builtins::get(filename)
            .ok_or_else(|| anyhow!("'{}' does not exist in @std", filename))?;
        let path = PathBuf::from(format!("/@std/{filename}"));
        let frozen = loader
            .eval_std_module(&path, content)
            .map_err(|e| anyhow!("{}", e))?;
        let name = filename.trim_end_matches(".axl").to_string();
        builtins.push((name, frozen.documentation()));
    }
    builtins.sort_by(|a, b| a.0.cmp(&b.0));
    Ok(Documentation { types, builtins })
}

/// Iterate over the embedded `@std//` builtin filenames (e.g. `"time.axl"`).
pub fn list_std_files() -> impl Iterator<Item = &'static str> {
    STD_DIR
        .files()
        .filter_map(|f| f.path().file_name()?.to_str())
        .filter(|name| name.ends_with(".axl"))
}
