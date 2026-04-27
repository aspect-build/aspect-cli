use anyhow::anyhow;
use starlark::environment::{FrozenModule, Globals, Module};
use starlark::eval::{Evaluator, FileLoader};
use starlark::syntax::{AstModule, Dialect};
use starlark::values::Value;
use std::cell::RefCell;
use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};

use super::api;
use super::error::EvalError;
use super::load_path::{LoadPath, join_confined};
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
pub struct AxlLoader {
    pub(super) cli_version: String,
    pub(super) repo_root: PathBuf,

    // The deps root directory where module expander expanded all the modules.
    pub(super) deps_root: PathBuf,

    pub(crate) dialect: Dialect,
    pub(crate) globals: Globals,

    // stack variables
    pub(crate) load_stack: RefCell<Vec<PathBuf>>,
    pub(crate) module_stack: RefCell<Vec<ModuleScope>>,

    loaded_modules: RefCell<HashMap<PathBuf, FrozenModule>>,
}

impl AxlLoader {
    pub fn new(cli_version: String, repo_root: PathBuf, deps_root: PathBuf) -> Self {
        Self {
            cli_version,
            repo_root,
            deps_root,
            dialect: api::dialect(),
            globals: api::get_globals().build(),
            load_stack: RefCell::new(vec![]),
            module_stack: RefCell::new(vec![]),
            loaded_modules: RefCell::new(HashMap::new()),
        }
    }

    pub fn new_store(&self, path: PathBuf) -> AxlStore {
        AxlStore::new(self.cli_version.clone(), self.repo_root.clone(), path)
    }

    /// Caches a frozen module by its absolute path so that subsequent `load()` calls
    /// for the same path return the cached module instead of re-evaluating.
    pub fn cache_module(&self, path: PathBuf, module: FrozenModule) {
        self.loaded_modules.borrow_mut().insert(path, module);
    }

    pub(super) fn eval_module(&self, path: &Path) -> Result<FrozenModule, EvalError> {
        self.eval_module_impl(path, None, false)
    }

    pub(crate) fn eval_std_module(
        &self,
        path: &Path,
        content: &'static str,
    ) -> Result<FrozenModule, EvalError> {
        self.eval_module_impl(path, Some(content), true)
    }

    fn eval_module_impl(
        &self,
        path: &Path,
        content: Option<&'static str>,
        is_std: bool,
    ) -> Result<FrozenModule, EvalError> {
        assert!(path.is_absolute());

        self.load_stack.borrow_mut().push(path.to_path_buf());

        let raw = match content {
            Some(s) => s.to_owned(),
            None => fs::read_to_string(&path)
                .map_err(|e| anyhow::anyhow!("{}: {}", path.display(), e))?,
        };

        let ast = AstModule::parse(&path.to_string_lossy(), raw, &self.dialect)?;
        let frozen = Module::with_temp_heap(|module| {
            if is_std {
                module.set("#_is_std#", Value::new_bool(true));
            }
            let store = self.new_store(path.to_path_buf());
            let mut eval = Evaluator::new(&module);
            eval.set_loader(self);
            eval.extra = Some(&store);
            eval.eval_module(ast, &self.globals)?;
            drop(eval);
            drop(store);
            module
                .freeze()
                .map_err(|e| EvalError::UnknownError(anyhow!("{:?}", e)))
        })?;

        self.load_stack.borrow_mut().pop();

        Ok(frozen)
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

impl FileLoader for AxlLoader {
    fn load(&self, raw: &str) -> starlark::Result<FrozenModule> {
        let load_path: LoadPath = raw.try_into()?;

        let load_stack = self.load_stack.borrow();
        let module_stack = self.module_stack.borrow();
        let parent_script_path = load_stack.last().expect("stack should not be empty");

        let module_info = module_stack
            .last()
            .expect("module name stack should not be empty");

        // Track whether we need to push/pop a new module scope for dependency loads.
        let new_module_scope = match &load_path {
            LoadPath::ModuleSpecifier { module, .. } => Some(ModuleScope {
                name: module.clone(),
                path: self.deps_root.join(module),
            }),
            _ => None,
        };

        // For @std// loads, resolve from the embedded dir; for others, from deps_root on disk.
        let (resolved_script_path, std_content) = match &load_path {
            LoadPath::ModuleSpecifier { module, subpath } if module == "std" => {
                let filename = subpath
                    .to_str()
                    .ok_or_else(|| anyhow!("invalid @std path: {:?}", subpath))?;
                let content = crate::builtins::get(filename)
                    .ok_or_else(|| anyhow!("'{}' does not exist in @std", filename))?;
                // Use a synthetic absolute path as the cache/stack key.
                let path = PathBuf::from(format!("/@std/{}", filename));
                (path, Some(content))
            }
            LoadPath::ModuleSpecifier { module, subpath } => {
                (self.resolve_in_deps_root(&module, &subpath)?, None)
            }
            LoadPath::ModuleSubpath(subpath) => (self.resolve(&module_info.path, subpath)?, None),
            LoadPath::RelativePath(relpath) => {
                let parent = parent_script_path.strip_prefix(&module_info.path).expect(
                    format!(
                        "parent script path {} should have same prefix as current module {}",
                        parent_script_path.display(),
                        module_info.path.display(),
                    )
                    .as_str(),
                );
                let path = if let Some(parent) = parent.parent() {
                    self.resolve(&module_info.path, &parent.join(relpath))?
                } else {
                    self.resolve(&module_info.path, relpath)?
                };
                (path, None)
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

        // If loading a dependency module, push its scope so relative imports resolve correctly.
        if let Some(scope) = &new_module_scope {
            drop(module_stack);
            self.module_stack.borrow_mut().push(scope.clone());
        } else {
            drop(module_stack);
        }

        let frozen_module = if let Some(content) = std_content {
            self.eval_std_module(&resolved_script_path, content)
        } else {
            self.eval_module(&resolved_script_path)
        }
        .map_err(|e| Into::<starlark::Error>::into(e))?;

        // Pop the dependency module scope if we pushed one.
        if new_module_scope.is_some() {
            self.module_stack.borrow_mut().pop();
        }

        self.loaded_modules
            .borrow_mut()
            .insert(resolved_script_path, frozen_module.clone());

        Ok(frozen_module)
    }
}
