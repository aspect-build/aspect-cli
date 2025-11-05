use std::cell::RefCell;
use std::collections::HashMap;
use std::fs;
use std::path::{Component, Path, PathBuf};

use anyhow::{anyhow, Context};
use starlark::environment::{FrozenModule, Module};
use starlark::eval::{Evaluator, FileLoader};
use starlark::syntax::AstModule;

use crate::eval::{AxlScriptEvaluator, LOAD_STACK};
use crate::helpers::sanitize_load_path_lexically;

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

    // The module root directory that relative loads cannot escape
    pub(super) module_root: PathBuf,

    // The deps root directory where module expander expanded all the modules.
    pub(super) axl_deps_root: PathBuf,
}

impl<'a> AxlLoader<'a> {
    fn resolve_axl_module_script(
        &self,
        module_name: &str,
        module_subpath: &Path,
    ) -> starlark::Result<(PathBuf, PathBuf)> {
        let module_root = self.axl_deps_root.join(module_name);
        let resolved_script_path = module_root.join(module_subpath);
        if resolved_script_path.is_file() {
            return Ok((resolved_script_path, module_root));
        }
        if !module_root.exists() {
            return Err(starlark::Error::new_other(anyhow!(
                "failed to resolve load(\"@{}/{}\", ...): module '{}' not found (expected module directory at '{}')",
                module_name,
                module_subpath.display(),
                module_name,
                module_root.display()
            )));
        } else if !module_root.is_dir() {
            return Err(starlark::Error::new_other(anyhow!(
                "failed to resolve load(\"@{}/{}\", ...): module '{}' root at '{}' exists but is not a directory",
                module_name,
                module_subpath.display(),
                module_name,
                module_root.display()
            )));
        } else {
            return Err(starlark::Error::new_other(anyhow!(
                "failed to resolve load(\"@{}/{}\", ...): script file not found in module '{}' (expected at '{}')",
                module_name,
                module_subpath.display(),
                module_name,
                resolved_script_path.display()
            )));
        }
    }

    fn resolve_axl_script(&self, script_path: &Path) -> starlark::Result<PathBuf> {
        let script_path_str = script_path.to_str().ok_or_else(|| {
            starlark::Error::new_other(anyhow!(
                "path is not valid UTF-8: {}",
                script_path.display()
            ))
        })?;
        let base: &PathBuf =
            if script_path_str.starts_with("./") || script_path_str.starts_with("../") {
                &self.script_dir
            } else {
                &self.module_root
            };

        let mut resolved_script_path = base.clone();
        for component in script_path.components() {
            match component {
                Component::CurDir => {}
                Component::ParentDir => {
                    resolved_script_path.pop();
                }
                Component::Normal(s) => {
                    resolved_script_path.push(s);
                }
                _ => {
                    return Err(starlark::Error::new_other(anyhow!(
                        "invalid path component in load path: {}",
                        script_path.display()
                    )));
                }
            }
        }

        if !resolved_script_path.starts_with(&self.module_root) {
            return Err(starlark::Error::new_other(anyhow!(
                "resolved path {} for load path {} escapes the module root directory {}",
                resolved_script_path.display(),
                script_path.display(),
                self.module_root.display()
            )));
        }

        Ok(resolved_script_path)
    }
}

impl<'a> FileLoader for AxlLoader<'a> {
    fn load(&self, load_path: &str) -> starlark::Result<FrozenModule> {
        if !self.script_dir.starts_with(&self.module_root) {
            return Err(starlark::Error::new_other(anyhow!(
                "script directory {} is not a descendant of module root directory {}",
                self.script_dir.display(),
                self.module_root.display()
            )));
        }

        let (module_name, module_subpath_or_script_path) = sanitize_load_path_lexically(load_path)?;

        // Check if the @module/path/to/file.axl has been loaded before; if it has use the cached version.
        // NB: it is by design that a `root/sub/.aspect/modules/foo/foo.axl` can override
        // the load of a `root/.aspect/modules/foo/foo.axl` and of `modules_cache/foo/foo.axl` since
        // we want to allow overriding of individual files in modules as needed.
        if module_name.is_some() {
            let module_specifier = format!(
                "@{}/{}",
                module_name.as_ref().unwrap(),
                module_subpath_or_script_path.display()
            );
            if let Some(module) =
                LOADED_MODULES.with(|modules| modules.borrow().get(&module_specifier).cloned())
            {
                return Ok(module);
            }
        }

        // Resolve the load path to a file on disk
        let (resolved_script_path, module_root) = if module_name.is_some() {
            self.resolve_axl_module_script(
                module_name.as_ref().unwrap().as_str(),
                &module_subpath_or_script_path,
            )?
        } else {
            (
                self.resolve_axl_script(&module_subpath_or_script_path)?,
                self.module_root.clone(),
            )
        };

        // Cycle detection: Prevent loading the same file recursively.
        let mut cycle_error = None;
        LOAD_STACK.with(|stack| {
            let mut s = stack.borrow_mut();
            if s.contains(&resolved_script_path) {
                let stack_str = s
                    .iter()
                    .map(|p| format!("- {}", p.display()))
                    .collect::<Vec<_>>()
                    .join("\n");
                cycle_error = Some(starlark::Error::new_other(anyhow!(
                    "cycle detected in load path:\n{}\n(cycles back to {})",
                    stack_str,
                    resolved_script_path.display()
                )));
            } else {
                s.push(resolved_script_path.clone());
            }
        });
        if let Some(err) = cycle_error {
            return Err(err);
        }

        // Read and parse the file content into an AST.
        let raw = fs::read_to_string(&resolved_script_path)
            .context(format!("failed to read {}", resolved_script_path.display()))?;
        let ast = AstModule::parse(
            &resolved_script_path.to_string_lossy(),
            raw,
            &self.script_evaluator.dialect,
        )?;

        // Set up a new module and evaluator for this file.
        let module = Module::new();
        let new_loader = AxlLoader {
            script_evaluator: self.script_evaluator,
            script_dir: resolved_script_path
                .parent()
                .expect("file path has parent")
                .to_path_buf(),
            module_root,
            axl_deps_root: self.axl_deps_root.clone(),
        };
        let mut eval = Evaluator::new(&module);
        eval.set_loader(&new_loader);
        eval.extra = Some(&self.script_evaluator.store);
        eval.eval_module(ast, &self.script_evaluator.globals)?;
        drop(eval);
        let frozen_module = module.freeze()?;

        // Cache the load @module/path/to/file.axl so it can be re-used on subsequent loads
        if module_name.is_some() {
            let module_specifier = format!(
                "@{}/{}",
                module_name.as_ref().unwrap(),
                module_subpath_or_script_path.display()
            );
            LOADED_MODULES.with(|modules| {
                modules
                    .borrow_mut()
                    .insert(module_specifier, frozen_module.clone());
            });
        }

        // Pop the load stack after successful load
        LOAD_STACK.with(|stack| {
            stack.borrow_mut().pop();
        });

        Ok(frozen_module)
    }
}
