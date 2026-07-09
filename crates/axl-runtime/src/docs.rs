//! Documentation extraction for tooling (e.g. `axl-docgen`).
//!
//! Bundles documentation for both the Rust-defined globals and the embedded
//! `@std//*.axl` builtin modules.

use crate::builtins::{self, BAZEL_DIR, STD_DIR};
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
    /// Documentation for each embedded builtin `.axl` module (`@std//*`
    /// and `@bazel//*`) as `(module, name, docs)` tuples, where `module`
    /// is the bare specifier (`"std"`, `"bazel"`) and `name` the basename
    /// without the `.axl` suffix (e.g. `"time"`, `"grpc"`).
    pub builtins: Vec<(String, String, DocModule)>,
}

/// Collect documentation for the entire AXL runtime — both Rust-defined types
/// and the embedded `@std//*.axl` builtin modules.
///
/// Must be called within an active Tokio runtime (the loader's `Env`
/// requires `tokio::runtime::Handle::current()`).
pub fn documentation() -> anyhow::Result<Documentation> {
    let types = get_globals().build().documentation();

    // One loader, reused across every `@std//*.axl` evaluation. `eval_std_module`
    // self-manages the load stack and never reads the module stack (std loads
    // short-circuit before any caller-context lookups in `FileLoader::load`),
    // so no seeding is required.
    let cwd = std::env::current_dir().unwrap_or_else(|_| PathBuf::from("/"));
    // Docgen only evaluates leaf `@std`/`@bazel` modules, which never consult
    // the root scope — an empty placeholder module satisfies the signature.
    let root_mod = crate::module::Mod::default();
    let loader = Loader::new("docgen".to_string(), cwd.clone(), cwd, None, &root_mod, &[]);

    let mut builtins = Vec::new();
    let modules = [
        ("std", list_std_files().collect::<Vec<_>>()),
        ("bazel", list_bazel_files().collect::<Vec<_>>()),
    ];
    for (module, filenames) in modules {
        for filename in filenames {
            let content = builtins::get(module, filename)
                .ok_or_else(|| anyhow!("'{}' does not exist in @{}", filename, module))?;
            let path = PathBuf::from(format!("/@{module}/{filename}"));
            let frozen = loader
                .eval_std_module(&path, content)
                .map_err(|e| anyhow!("{}", e))?;
            let name = filename.trim_end_matches(".axl").to_string();
            builtins.push((module.to_string(), name, frozen.documentation()));
        }
    }
    builtins.sort_by(|a, b| a.1.cmp(&b.1));
    Ok(Documentation { types, builtins })
}

/// Iterate over the embedded `@std//` builtin filenames (e.g. `"time.axl"`).
pub fn list_std_files() -> impl Iterator<Item = &'static str> {
    STD_DIR
        .files()
        .filter_map(|f| f.path().file_name()?.to_str())
        .filter(|name| name.ends_with(".axl"))
}

/// Iterate over the embedded `@bazel//` builtin filenames (e.g.
/// `"grpc.axl"`). The synthetic `proto/*.axl` shims are not included —
/// they're one-line re-exports of `_proto.<pkg>` namespaces whose docs
/// come from the Rust-defined globals.
pub fn list_bazel_files() -> impl Iterator<Item = &'static str> {
    BAZEL_DIR
        .files()
        .filter_map(|f| f.path().file_name()?.to_str())
        .filter(|name| name.ends_with(".axl"))
}
