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
use crate::engine::store::Env;
use crate::module::Mod;

/// State of a path in the loader's module cache.
///
/// `Loading` is set on entry to evaluation and replaced with `Loaded` on success
/// (or removed on failure). Hitting `Loading` during a recursive `load()` is
/// definitionally a cycle.
#[derive(Debug)]
enum LoadState {
    Loading,
    Loaded(FrozenModule),
}

/// Internal loader for .axl files, handling path resolution, security checks, and recursive loading.
#[derive(Debug)]
pub struct AxlLoader<'m> {
    pub(crate) env: Env,

    dialect: Dialect,
    pub(crate) globals: Globals,

    /// All modules (root + deps) discovered during MODULE.aspect expansion,
    /// looked up by name to resolve `@<name>//path` loads.
    modules: &'m [Mod],

    /// Stack of files currently being evaluated. Top is the parent of any
    /// `load()` triggered during evaluation. Used for relative-path resolution
    /// and for rendering cycle traces.
    load_stack: RefCell<Vec<PathBuf>>,

    /// Stack of modules currently in scope. Pushed by `eval_module` for the
    /// outermost call and again on every recursive `load()` (whether the
    /// target is the same module or a `@dep//` crossing).
    module_stack: RefCell<Vec<&'m Mod>>,

    /// Module cache. Doubles as the cycle detector: an entry of `Loading` means
    /// the path is mid-evaluation on the current stack.
    loaded_modules: RefCell<HashMap<PathBuf, LoadState>>,
}

impl<'m> AxlLoader<'m> {
    pub fn new(cli_version: String, repo_root: PathBuf, modules: &'m [Mod]) -> Self {
        Self {
            env: Env::new(cli_version, repo_root),
            dialect: api::dialect(),
            globals: api::get_globals().build(),
            modules,
            load_stack: RefCell::new(vec![]),
            module_stack: RefCell::new(vec![]),
            loaded_modules: RefCell::new(HashMap::new()),
        }
    }

    /// Evaluate a `.axl` file in the given module's scope. Pushes `scope` onto
    /// the module stack for the duration so any nested relative loads resolve
    /// against `scope.root`. Callers never touch `module_stack` directly.
    pub(crate) fn eval_module(
        &self,
        scope: &'m Mod,
        path: &Path,
    ) -> Result<FrozenModule, EvalError> {
        self.module_stack.borrow_mut().push(scope);
        let result = self.eval_module_impl(path, None, false);
        self.module_stack.borrow_mut().pop();
        result
    }

    /// Evaluate an embedded `@std//` file. Std files are leaves — they don't
    /// take a `Mod` scope because they only `load("@std//…")` or `load("@dep//…")`,
    /// never relative paths.
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

        match self.loaded_modules.borrow().get(path) {
            Some(LoadState::Loaded(m)) => return Ok(m.clone()),
            Some(LoadState::Loading) => return Err(self.cycle_error(path)),
            None => {}
        }

        self.loaded_modules
            .borrow_mut()
            .insert(path.to_path_buf(), LoadState::Loading);
        self.load_stack.borrow_mut().push(path.to_path_buf());

        let result = self.eval_module_inner(path, content, is_std);

        self.load_stack.borrow_mut().pop();

        match result {
            Ok(frozen) => {
                self.loaded_modules
                    .borrow_mut()
                    .insert(path.to_path_buf(), LoadState::Loaded(frozen.clone()));
                Ok(frozen)
            }
            Err(e) => {
                self.loaded_modules.borrow_mut().remove(path);
                Err(e)
            }
        }
    }

    fn eval_module_inner(
        &self,
        path: &Path,
        content: Option<&'static str>,
        is_std: bool,
    ) -> Result<FrozenModule, EvalError> {
        let raw = match content {
            Some(s) => s.to_owned(),
            None => fs::read_to_string(path)
                .map_err(|e| anyhow::anyhow!("{}: {}", path.display(), e))?,
        };

        let ast = AstModule::parse(&path.to_string_lossy(), raw, &self.dialect)?;
        Module::with_temp_heap(|module| {
            if is_std {
                module.set("#_is_std#", Value::new_bool(true));
            }
            let mut eval = Evaluator::new(&module);
            eval.set_loader(self);
            eval.extra = Some(&self.env);
            eval.eval_module(ast, &self.globals)?;
            drop(eval);
            module
                .freeze()
                .map_err(|e| EvalError::UnknownError(anyhow!("{:?}", e)))
        })
    }

    fn cycle_error(&self, target: &Path) -> EvalError {
        let stack = self.load_stack.borrow();
        let trace = stack
            .iter()
            .map(|p| format!("- {}", p.display()))
            .collect::<Vec<_>>()
            .join("\n");
        EvalError::UnknownError(anyhow!(
            "cycle detected in load path:\n{}\n(cycles back to {:?})",
            trace,
            target
        ))
    }

    fn lookup_module(&self, name: &str) -> anyhow::Result<&'m Mod> {
        self.modules
            .iter()
            .find(|m| m.name == name)
            .ok_or_else(|| anyhow!("module '{}' is not declared.", name))
    }
}

impl<'m> AxlLoader<'m> {
    /// Top of the module stack — the scope of the file currently evaluating.
    /// Errors if the stack is empty (callers reach this only via load arms
    /// that genuinely need a parent module).
    fn current_module(&self) -> anyhow::Result<&'m Mod> {
        let stack = self.module_stack.borrow();
        stack.last().copied().ok_or_else(|| {
            anyhow!("no module on the stack — relative or subpath load with no caller scope")
        })
    }

    /// Top of the load stack — the file currently evaluating. Used only for
    /// resolving the directory of `RelativePath` loads.
    fn current_script(&self) -> anyhow::Result<PathBuf> {
        let stack = self.load_stack.borrow();
        stack.last().cloned().ok_or_else(|| {
            anyhow!("no script on the load stack — relative load with no caller context")
        })
    }
}

impl<'m> FileLoader for AxlLoader<'m> {
    fn load(&self, raw: &str) -> starlark::Result<FrozenModule> {
        let load_path: LoadPath = raw.try_into()?;

        // Each arm yields the resolved path plus either embedded std content
        // or a `Mod` scope to evaluate under. Std loads carry no scope (they're
        // leaves); every other shape inherits or crosses into a `Mod`.
        enum Target<'m> {
            Std(&'static str),
            Module(&'m Mod),
        }

        let (resolved_script_path, target): (PathBuf, Target<'m>) = match &load_path {
            LoadPath::ModuleSpecifier { module, subpath } if module == "std" => {
                let filename = subpath
                    .to_str()
                    .ok_or_else(|| anyhow!("invalid @std path: {:?}", subpath))?;
                let content = crate::builtins::get(filename)
                    .ok_or_else(|| anyhow!("'{}' does not exist in @std", filename))?;
                let path = PathBuf::from(format!("/@std/{}", filename));
                (path, Target::Std(content))
            }
            LoadPath::ModuleSpecifier { module, subpath } => {
                let dep = self.lookup_module(module)?;
                let path = join_confined(&dep.root, subpath)?;
                if !path.is_file() {
                    return Err(starlark::Error::new_other(anyhow!(
                        "path {:?} does not exist in module `{}`.",
                        subpath,
                        module
                    )));
                }
                (path, Target::Module(dep))
            }
            LoadPath::ModuleSubpath(subpath) => {
                let scope = self.current_module()?;
                (join_confined(&scope.root, subpath)?, Target::Module(scope))
            }
            LoadPath::RelativePath(relpath) => {
                let scope = self.current_module()?;
                let parent_script = self.current_script()?;
                let parent_in_module = parent_script.strip_prefix(&scope.root).map_err(|_| {
                    anyhow!(
                        "parent script {} is not within current module {}",
                        parent_script.display(),
                        scope.root.display()
                    )
                })?;
                let path = if let Some(parent_dir) = parent_in_module.parent() {
                    join_confined(&scope.root, &parent_dir.join(relpath))?
                } else {
                    join_confined(&scope.root, relpath)?
                };
                (path, Target::Module(scope))
            }
        };

        let frozen = match target {
            Target::Std(content) => self.eval_std_module(&resolved_script_path, content),
            Target::Module(scope) => self.eval_module(scope, &resolved_script_path),
        };

        frozen.map_err(|e| Into::<starlark::Error>::into(e))
    }
}
