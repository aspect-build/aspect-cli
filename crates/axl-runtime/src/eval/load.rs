use anyhow::anyhow;
use starlark::environment::{FrozenModule, Globals, Module};
use starlark::eval::{Evaluator, FileLoader};
use starlark::syntax::{AstModule, Dialect};
use std::cell::RefCell;
use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};

use super::api;
use super::error::EvalError;
use super::load_path::{join_confined, LoadPath};
use crate::engine::store::AxlStore;

#[derive(Debug, Clone)]
pub struct ModuleScope {
    // The current module name that the load statement is in
    pub name: String,

    // The module root directory that relative loads cannot escape
    pub path: PathBuf,
}

/// Internal loader for .axl files, handling path resolution, security checks, and recursive loading.
#[derive(Debug)]
pub struct AxlLoader<'p> {
    // The deps root directory where module expander expanded all the modules.
    pub(super) deps_root: &'p PathBuf,
    pub(super) store: AxlStore,

    pub(crate) dialect: Dialect,
    pub(crate) globals: Globals,

    // stack variables
    pub(crate) load_stack: RefCell<Vec<PathBuf>>,

    pub(crate) module_stack: RefCell<Vec<ModuleScope>>,

    loaded_modules: RefCell<HashMap<PathBuf, FrozenModule>>,
}

impl<'p> AxlLoader<'p> {
    pub fn new(deps_root: &'p PathBuf) -> Self {
        Self {
            deps_root,
            dialect: api::dialect(),
            globals: api::get_globals().build(),
            store: AxlStore::new(),
            load_stack: RefCell::new(vec![]),
            module_stack: RefCell::new(vec![]),
            loaded_modules: RefCell::new(HashMap::new()),
        }
    }

    pub fn store(&self) -> &AxlStore {
        &self.store
    }

    pub(super) fn eval_module(&self, path: &Path) -> Result<Module, EvalError> {
        assert!(path.is_absolute());

        // Push the script path onto the LOAD_STACK (used to detect circular loads)
        self.load_stack.borrow_mut().push(path.to_path_buf());
        // Load and evaluate the script
        let raw = fs::read_to_string(&path)?;
        let ast = AstModule::parse(&path.to_string_lossy(), raw, &self.dialect)?;
        let module = Module::new();
        let mut eval = Evaluator::new(&module);
        eval.set_loader(self);
        eval.extra = Some(&self.store);
        eval.eval_module(ast, &self.globals)?;

        drop(eval);

        // Pop the script path off of the LOAD_STACK
        self.load_stack.borrow_mut().pop();

        // Return the evaluated script
        Ok(module)
    }

    fn resolve_in_deps_root(
        &self,
        module_name: &str,
        module_subpath: &Path,
    ) -> anyhow::Result<PathBuf> {
        let module_root = self.deps_root.join(module_name);

        if !module_root.exists() {
            return Err(anyhow!("module '{}' is not declared.", module_name,));
        }

        let resolved_path = join_confined(&module_root, module_subpath)?;

        if !resolved_path.is_file() {
            return Err(anyhow!(
                "path '{:?}' does not exist in module `{}`.",
                module_subpath,
                module_name,
            ));
        }

        Ok(resolved_path)
    }

    fn resolve(&self, module_root: &Path, subpath: &Path) -> anyhow::Result<PathBuf> {
        join_confined(&module_root, subpath)
    }
}

impl<'p> FileLoader for AxlLoader<'p> {
    fn load(&self, raw: &str) -> starlark::Result<FrozenModule> {
        let load_path: LoadPath = raw.try_into()?;

        let load_stack = self.load_stack.borrow();
        let module_stack = self.module_stack.borrow();
        let parent_script_path = load_stack.last().expect("stack should not be empty");

        let module_info = module_stack
            .last()
            .expect("module name stack should not be empty");

        let resolved_script_path = match &load_path {
            LoadPath::ModuleSpecifier { module, subpath } => {
                self.resolve_in_deps_root(&module, &subpath)?
            }
            LoadPath::ModuleSubpath(subpath) => self.resolve(&module_info.path, subpath)?,
            LoadPath::RelativePath(relpath) => {
                let parent = parent_script_path
                    .strip_prefix(&module_info.path)
                    .expect("parent script path should have same prefix as current module");
                if let Some(parent) = parent.parent() {
                    self.resolve(&module_info.path, &parent.join(relpath))?
                } else {
                    self.resolve(&module_info.path, relpath)?
                }
            }
        };

        // If the module is already loaded, then just return it.
        if let Some(cached_module) = self
            .loaded_modules
            .borrow()
            .get(&resolved_script_path)
            .cloned()
        {
            return Ok(cached_module);
        }

        // Detect cycles and prevent loading the same file recursively.
        if load_stack.contains(&resolved_script_path) {
            let stack_str = load_stack
                .iter()
                .map(|p| format!("- {}", p.display()))
                .collect::<Vec<_>>()
                .join("\n");
            return Err(starlark::Error::new_other(anyhow!(
                "cycle detected in load path:\n{}\n(cycles back to {:?})",
                stack_str,
                resolved_script_path
            )));
        }

        drop(load_stack);

        // Push the resolved path to the stack so that relative imports from the file still works.
        // load_stack.push(resolved_script_path.clone());

        // Read and parse the file content into an AST.
        let frozen_module = self
            .eval_module(&resolved_script_path)
            .map_err(|e| Into::<starlark::Error>::into(e))?
            .freeze()?;

        // Pop the load stack after successful load
        // self.load_stack.borrow_mut().pop();

        // Cache the load @module//path/to/file.axl so it can be re-used on subsequent loads
        self.loaded_modules
            .borrow_mut()
            .insert(resolved_script_path, frozen_module.clone());

        Ok(frozen_module)
    }
}
