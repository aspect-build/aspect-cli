use anyhow::anyhow;
use starlark::environment::{FrozenModule, Globals, Module};
use starlark::eval::{Evaluator, FileLoader};
use starlark::syntax::{AstModule, Dialect};
use starlark::values::{OwnedFrozenValue, Value, ValueLike};
use std::cell::RefCell;
use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use super::api;
use super::error::EvalError;
use super::load_path::{LoadPath, join_confined};
use super::prefetch::{Prefetch, Taken};
use crate::engine::store::Env;
use crate::engine::r#trait::FrozenTraitType;
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

    /// The parse-ahead running for the current phase, if any; see
    /// [`Self::with_prefetch`].
    prefetch: RefCell<Option<Arc<Prefetch<'m>>>>,
}

/// Where a `load()` resolves to: embedded `@std`/`@bazel` content, or a file
/// evaluated under a `Mod` scope.
pub(super) enum Target<'m> {
    Std(&'static str),
    Module(&'m Mod),
}

impl<'m> AxlLoader<'m> {
    pub fn new(
        cli_version: String,
        aspect_root: PathBuf,
        bazel_root: PathBuf,
        git_root: Option<PathBuf>,
        modules: &'m [Mod],
    ) -> Self {
        Self {
            env: Env::new(cli_version, aspect_root, bazel_root, git_root),
            dialect: api::dialect(),
            globals: api::get_globals().build(),
            modules,
            load_stack: RefCell::new(vec![]),
            module_stack: RefCell::new(vec![]),
            loaded_modules: RefCell::new(HashMap::new()),
            prefetch: RefCell::new(None),
        }
    }

    /// Run `body` while files reachable from `roots` are parsed ahead of
    /// their evaluation on other threads; see [`super::prefetch`].
    pub(crate) fn with_prefetch<T>(
        &self,
        roots: Vec<(&'m Mod, PathBuf)>,
        body: impl FnOnce() -> T,
    ) -> T {
        super::prefetch::with_prefetch(self.modules, &self.dialect, roots, |prefetch| {
            let previous = self.prefetch.replace(prefetch);
            let out = body();
            *self.prefetch.borrow_mut() = previous;
            out
        })
    }

    /// The value `@<module>//<subpath>` exports as `name`, if that file has
    /// been loaded. Never loads it.
    pub(crate) fn loaded_export(
        &self,
        module: &str,
        subpath: &str,
        name: &str,
    ) -> Option<OwnedFrozenValue> {
        let dep = self.modules.iter().find(|m| m.name == module)?;
        let path = join_confined(&dep.root, Path::new(subpath)).ok()?;
        match self.loaded_modules.borrow().get(&path)? {
            LoadState::Loaded(m) => m.get(name).ok(),
            LoadState::Loading => None,
        }
    }

    /// The first trait type a loaded module exports for which `pred` holds,
    /// visiting modules in path order so the pick is deterministic.
    pub(crate) fn find_exported_trait(
        &self,
        pred: impl Fn(&FrozenTraitType) -> bool,
    ) -> Option<OwnedFrozenValue> {
        let loaded = self.loaded_modules.borrow();
        let mut modules: Vec<(&PathBuf, &FrozenModule)> = loaded
            .iter()
            .filter_map(|(path, state)| match state {
                LoadState::Loaded(m) => Some((path, m)),
                LoadState::Loading => None,
            })
            .collect();
        modules.sort_by(|a, b| a.0.cmp(b.0));
        modules.into_iter().find_map(|(_, m)| {
            m.names()
                .filter_map(|name| m.get(name.as_str()).ok())
                .find(|v| {
                    v.value()
                        .downcast_ref::<FrozenTraitType>()
                        .is_some_and(&pred)
                })
        })
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
        let prefetch = self.prefetch.borrow().clone();
        let scope = self.module_stack.borrow().last().copied();
        let prefetched = match (&prefetch, scope, content) {
            (Some(prefetch), Some(scope), None) => prefetch.take(scope, path),
            _ => Taken::Absent,
        };
        let ast = match prefetched {
            Taken::Parsed(ast) => ast,
            Taken::Absent => {
                let raw = match content {
                    Some(s) => s.to_owned(),
                    None => fs::read_to_string(path)
                        .map_err(|e| anyhow::anyhow!("{}: {}", path.display(), e))?,
                };
                AstModule::parse(&path.to_string_lossy(), raw, &self.dialect)?
            }
        };
        Module::with_temp_heap(|module| {
            if is_std {
                module.set("#_is_std#", Value::new_bool(true));
            }
            let mut eval = Evaluator::new(&module);
            eval.set_print_handler(&crate::out::TOLERANT_PRINT_HANDLER);
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
}

impl<'m> FileLoader for AxlLoader<'m> {
    fn load(&self, raw: &str) -> starlark::Result<FrozenModule> {
        let load_path: LoadPath = raw.try_into()?;
        let scope = self.module_stack.borrow().last().copied();
        let (resolved_script_path, target) = {
            let load_stack = self.load_stack.borrow();
            let parent = load_stack.last().map(PathBuf::as_path);
            resolve(self.modules, &load_path, scope, parent)?
        };

        let frozen = match target {
            Target::Std(content) => self.eval_std_module(&resolved_script_path, content),
            Target::Module(scope) => self.eval_module(scope, &resolved_script_path),
        };

        frozen.map_err(|e| Into::<starlark::Error>::into(e))
    }
}

/// Resolve `load_path`, loaded from `parent` under module `scope`, to the file
/// it names and how to evaluate it. Shared by [`AxlLoader`]'s `load()` and
/// [`super::prefetch`], so the two cannot disagree about which file a
/// load means.
pub(super) fn resolve<'m>(
    modules: &'m [Mod],
    load_path: &LoadPath,
    scope: Option<&'m Mod>,
    parent: Option<&Path>,
) -> anyhow::Result<(PathBuf, Target<'m>)> {
    // Only the relative arms need a caller.
    let scope = || {
        scope.ok_or_else(|| {
            anyhow!("no module on the stack — relative or subpath load with no caller scope")
        })
    };
    Ok(match load_path {
        LoadPath::ModuleSpecifier { module, subpath } if module == "std" || module == "bazel" => {
            let filename = subpath
                .to_str()
                .ok_or_else(|| anyhow!("invalid @{} path: {:?}", module, subpath))?;
            let content = crate::builtins::get(module, filename)
                .ok_or_else(|| anyhow!("'{}' does not exist in @{}", filename, module))?;
            let path = PathBuf::from(format!("/@{}/{}", module, filename));
            (path, Target::Std(content))
        }
        LoadPath::ModuleSpecifier { module, subpath } => {
            let dep = modules
                .iter()
                .find(|m| &m.name == module)
                .ok_or_else(|| anyhow!("module '{}' is not declared.", module))?;
            let path = join_confined(&dep.root, subpath)?;
            if !path.is_file() {
                return Err(anyhow!(
                    "path {:?} does not exist in module `{}`.",
                    subpath,
                    module
                ));
            }
            (path, Target::Module(dep))
        }
        LoadPath::ModuleSubpath(subpath) => {
            let scope = scope()?;
            (join_confined(&scope.root, subpath)?, Target::Module(scope))
        }
        LoadPath::RelativePath(relpath) => {
            let scope = scope()?;
            let parent_script = parent.ok_or_else(|| {
                anyhow!("no script on the load stack — relative load with no caller context")
            })?;
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
    })
}
