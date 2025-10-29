use std::cell::RefCell;
use std::collections::HashMap;
use std::ffi::OsStr;
use std::fs;
use std::path::{Component, Path, PathBuf};

use anyhow::{anyhow, Context};
use starlark::environment::{FrozenModule, Module};
use starlark::eval::{Evaluator, FileLoader};
use starlark::syntax::AstModule;

use crate::engine::context::AxlContext;
use crate::eval::{is_local_module_path, AxlScriptEvaluator, LOAD_STACK};
use crate::helpers::{sanitize_load_path_lexically, ASPECT_ROOT, AXL_MODULE_DIR};

thread_local! {
    static LOADED_MODULES: RefCell<HashMap<String, FrozenModule>> = RefCell::new(HashMap::new());
}

/// Internal loader for .axl files, handling path resolution, security checks, and recursive loading.
#[derive(Debug)]
pub struct AxlLoader<'a> {
    // The script evaluator
    pub(super) script_evaluator: &'a AxlScriptEvaluator,

    // The script dir which relative loads are relative to
    pub(super) script_dir: PathBuf,

    // Whether the script is in a module
    pub(super) in_module: bool,

    // The root directory that relative loads cannot escape
    pub(super) root_dir: PathBuf,

    // The root directory that remote deps live in.
    pub(super) root_deps_dir: PathBuf,
}

impl<'a> AxlLoader<'a> {
    fn find_nearest_aspect_dir_parent(&self, path: &Path) -> Option<PathBuf> {
        let mut current = path.to_path_buf();
        while let Some(parent) = current.parent() {
            if current.file_name() == Some(OsStr::new(ASPECT_ROOT)) {
                return Some(parent.to_path_buf());
            }
            current = parent.to_path_buf();
        }
        None
    }

    fn resolve_module(
        &self,
        module_name: &str,
        load_path: &Path,
    ) -> starlark::Result<(PathBuf, PathBuf)> {
        let mut search_dirs: Vec<PathBuf> = Vec::new();

        if self.in_module {
            // Add the module root dir
            search_dirs.push(self.root_dir.clone());
        } else {
            // Check if script_dir is within an .aspect folder
            let nearest_opt = self.find_nearest_aspect_dir_parent(&self.script_dir);
            let mut current: Option<&Path>;
            if let Some(ref nearest) = nearest_opt {
                search_dirs.push(
                    nearest
                        .join(ASPECT_ROOT)
                        .join(AXL_MODULE_DIR)
                        .join(module_name),
                );
                current = nearest.parent();
            } else {
                search_dirs.push(
                    self.script_dir
                        .join(ASPECT_ROOT)
                        .join(AXL_MODULE_DIR)
                        .join(module_name),
                );
                current = self.script_dir.parent();
            }

            // Add for each parent up to but not including root_dir
            while let Some(parent) = current {
                if !parent.starts_with(&self.root_dir) {
                    break;
                }
                search_dirs.push(
                    parent
                        .join(ASPECT_ROOT)
                        .join(AXL_MODULE_DIR)
                        .join(module_name),
                );
                current = parent.parent();
            }
        }

        // Add cache_dir_for_module
        let cache_dir_for_module = self.root_deps_dir.join(module_name);
        if self.root_dir != cache_dir_for_module {
            search_dirs.push(cache_dir_for_module);
        }

        let mut search_paths: Vec<PathBuf> = Vec::new();
        for dir in &search_dirs {
            let path = dir.join(load_path);
            search_paths.push(path.clone());
            if path.is_file() {
                return Ok((path, dir.clone()));
            }
        }

        let searched_str = search_paths
            .iter()
            .map(|p| format!(" - {}", p.display()))
            .collect::<Vec<_>>()
            .join("\n");
        Err(starlark::Error::new_other(anyhow!(
            "Module load not found for @{}/{};\nsearched:\n{}",
            module_name,
            load_path.display(),
            searched_str
        )))
    }

    fn resolve(&self, load_path: &Path) -> starlark::Result<PathBuf> {
        let load_str = load_path.to_str().ok_or_else(|| {
            starlark::Error::new_other(anyhow!("Path is not valid UTF-8: {}", load_path.display()))
        })?;
        let rel_path = load_path;
        let base = if load_str.starts_with("./") || load_str.starts_with("../") {
            &self.script_dir
        } else {
            &self.root_dir
        };

        let mut full_path = base.clone();
        for component in rel_path.components() {
            match component {
                Component::CurDir => {}
                Component::ParentDir => {
                    full_path.pop();
                }
                Component::Normal(s) => {
                    full_path.push(s);
                }
                _ => {
                    return Err(starlark::Error::new_other(anyhow!(
                        "Invalid path component in load_path: {}",
                        load_path.display()
                    )));
                }
            }
        }

        if !full_path.starts_with(&self.root_dir) {
            return Err(starlark::Error::new_other(anyhow!(
                "Resolved path {} for load_path {} escapes the root_dir {}",
                full_path.display(),
                load_path.display(),
                self.root_dir.display()
            )));
        }

        Ok(full_path)
    }
}

impl<'a> FileLoader for AxlLoader<'a> {
    fn load(&self, load_path: &str) -> starlark::Result<FrozenModule> {
        if !self.script_dir.starts_with(&self.root_dir) {
            return Err(starlark::Error::new_other(anyhow!(
                "script_dir {} is not a descendant of root_dir {}",
                self.script_dir.display(),
                self.root_dir.display()
            )));
        }

        let (module_name, load_path) = sanitize_load_path_lexically(load_path)?;

        // Don't allow loading via .aspect/modules/foo/path/to/file.axl -- force use of @foo/path/to/file.axl
        // so we can check the LOADED_MODULES cache
        if is_local_module_path(&load_path) {
            return Err(starlark::Error::new_other(anyhow::anyhow!(
                "Cannot load AXL files directly from local module directories; use the @module_name/ syntax instead: {}",
                load_path.display(),
            )));
        }

        // Check if the @module/path/to/file.axl has been loaded before; if it has use the cached version.
        // NB: it is by design that a `root/sub/.aspect/modules/foo/foo.axl` can override
        // the load of a `root/.aspect/modules/foo/foo.axl` and of `modules_cache/foo/foo.axl` since
        // we want to allow overriding of individual files in modules as needed.
        if module_name.is_some() {
            let module_path = format!("@{}/{}", module_name.as_ref().unwrap(), load_path.display());
            if let Some(module) =
                LOADED_MODULES.with(|modules| modules.borrow().get(&module_path).cloned())
            {
                return Ok(module);
            }
        }

        // Resolve the load path to a file on disk
        let (resolved, root_dir) = if module_name.is_some() {
            self.resolve_module(module_name.as_ref().unwrap().as_str(), &load_path)?
        } else {
            (self.resolve(&load_path)?, self.root_dir.clone())
        };

        // Cycle detection: Prevent loading the same file recursively.
        let mut cycle_error = None;
        LOAD_STACK.with(|stack| {
            let mut s = stack.borrow_mut();
            if s.contains(&resolved) {
                let stack_str = s
                    .iter()
                    .map(|p| format!("- {}", p.display()))
                    .collect::<Vec<_>>()
                    .join("\n");
                cycle_error = Some(starlark::Error::new_other(anyhow!(
                    "Cycle detected in load path:\n{}\n(cycles back to {})",
                    stack_str,
                    resolved.display()
                )));
            } else {
                s.push(resolved.clone());
            }
        });
        if let Some(err) = cycle_error {
            return Err(err);
        }

        // Read and parse the file content into an AST.
        let raw = fs::read_to_string(&resolved)
            .context(format!("failed to read {}", resolved.display()))?;
        let ast = AstModule::parse(
            &resolved.to_string_lossy(),
            raw,
            &self.script_evaluator.dialect,
        )?;

        // Set up a new module and evaluator for this file.
        let module = Module::new();
        let new_loader = AxlLoader {
            script_evaluator: self.script_evaluator,
            script_dir: resolved
                .parent()
                .expect("file path has parent")
                .to_path_buf(),
            in_module: module_name.is_some(),
            root_dir,
            root_deps_dir: self.root_deps_dir.clone(),
        };
        let ctx = AxlContext {
            runtime: self.script_evaluator.async_runtime.clone(),
            tools: HashMap::new(),
        };
        let mut eval = Evaluator::new(&module);
        eval.set_loader(&new_loader);
        eval.extra = Some(&ctx);
        eval.eval_module(ast, &self.script_evaluator.globals)?;
        drop(eval);
        let frozen_module = module.freeze()?;

        // Cache the load @module/path/to/file.axl so it can be re-used on subsequent loads
        if module_name.is_some() {
            let module_path = format!("@{}/{}", module_name.as_ref().unwrap(), load_path.display());
            LOADED_MODULES.with(|modules| {
                modules
                    .borrow_mut()
                    .insert(module_path, frozen_module.clone());
            });
        }

        // Pop the load stack after successful load
        LOAD_STACK.with(|stack| {
            stack.borrow_mut().pop();
        });

        Ok(frozen_module)
    }
}
