mod load;

use std::cell::RefCell;
use std::ffi::OsStr;
use std::fs;
use std::path::{Component, Path, PathBuf};
use std::rc::Rc;

use anyhow::anyhow;
use starlark::environment::{Globals, GlobalsBuilder, LibraryExtension, Module};
use starlark::eval::Evaluator;
use starlark::syntax::{AstModule, Dialect, DialectTypes};
use starlark::values::{Heap, ValueLike};
use thiserror::Error;

use crate::engine::r#async::rt::AsyncRuntime;
use crate::engine::task::{AsTaskLike, FrozenTask, TaskLike};
use crate::engine::task_args::TaskArgs;
use crate::engine::{self, task::Task};
use crate::eval::load::AxlLoader;
use crate::helpers::{
    normalize_abs_path_lexically, sanitize_load_path_lexically, ASPECT_ROOT,
};

/// The core evaluator for .axl files, holding configuration like repository root,
/// Starlark dialect, globals, and async runtime. Used to evaluate .axl files securely.
#[derive(Debug)]
pub struct AxlScriptEvaluator {
    // Repo root is where the module boundary is.
    repo_root: PathBuf,
    // Deps root is where module expander expanded all the modules.
    deps_root: PathBuf,
    dialect: Dialect,
    globals: Globals,
    async_runtime: AsyncRuntime,
}

/// Represents the result of evaluating an .axl script, encapsulating the module,
/// path, and runtime for task definition retrieval and execution.
#[derive(Debug, Clone)]
pub struct EvaluatedAxlScript {
    pub path: PathBuf,
    pub module: Rc<Module>,
    async_runtime: AsyncRuntime,
}

/// Enum representing possible errors during evaluation, including Starlark-specific errors,
/// missing symbols, and wrapped anyhow or IO errors.
#[derive(Error, Debug)]
#[non_exhaustive]
pub enum EvalError {
    #[error("{0}")]
    StarlarkError(starlark::Error),
    #[error("task file {0} does not export the `{1}` symbol")]
    MissingSymbol(PathBuf, String),
    #[error(transparent)]
    UnknownError(#[from] anyhow::Error),
    #[error(transparent)]
    IOError(#[from] std::io::Error),
}

thread_local! {
    static LOAD_STACK: RefCell<Vec<PathBuf>> = RefCell::new(Vec::new());
}

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
    engine::register_toplevels(&mut globals);
    globals
}

// Custom From implementation since starlark::Error doesn't implement std::error::Error.
impl From<starlark::Error> for EvalError {
    fn from(value: starlark::Error) -> Self {
        Self::StarlarkError(value)
    }
}

impl EvaluatedAxlScript {
    fn new(path: PathBuf, async_runtime: AsyncRuntime, module: Module) -> Self {
        Self {
            path,
            module: Rc::new(module),
            async_runtime,
        }
    }

    /// Retrieves a task definition from the evaluated module by symbol name.
    pub fn definition(&self, symbol: &str) -> Result<&dyn TaskLike, EvalError> {
        let def = self.module.get(symbol).ok_or(EvalError::MissingSymbol(
            self.path.clone(),
            symbol.to_string(),
        ))?;
        if let Some(task) = def.downcast_ref::<Task>() {
            return Ok(task.as_task());
        } else if let Some(task) = def.downcast_ref::<FrozenTask>() {
            return Ok(task.as_task());
        } else {
            return Err(EvalError::UnknownError(anyhow!("expected type of Task")));
        }
    }

    /// Executes a task from the module by symbol, providing arguments and returning the exit code.
    pub fn execute(
        &self,
        symbol: &str,
        args: impl FnOnce(&Heap) -> TaskArgs,
    ) -> Result<Option<u8>, EvalError> {
        let def = self.module.get(symbol).ok_or(EvalError::MissingSymbol(
            self.path.clone(),
            symbol.to_string(),
        ))?;

        let heap = self.module.heap();
        let args = args(heap);
        let context = heap.alloc(engine::task_context::TaskContext::new(args));
        let mut eval = Evaluator::new(&self.module);
        eval.extra = Some(&self.async_runtime);
        let ret = if let Some(val) = def.downcast_ref::<Task>() {
            eval.eval_function(val.implementation(), &[context], &[])?
        } else if let Some(val) = def.downcast_ref::<FrozenTask>() {
            eval.eval_function(val.implementation().to_value(), &[context], &[])?
        } else {
            return Err(EvalError::UnknownError(anyhow::anyhow!(
                "expected value of type `Task`"
            )));
        };
        Ok(ret.unpack_i32().map(|ex| ex as u8))
    }
}

impl AxlScriptEvaluator {
    /// Returns the configured Starlark dialect for .axl files.
    fn dialect() -> Dialect {
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

    /// Returns the globals used for evaluation.
    fn globals() -> Globals {
        get_globals().build()
    }

    /// Creates a new AxlScriptEvaluator with the given repository root.
    pub fn new(repo_root: PathBuf, deps_root: PathBuf) -> Self {
        Self {
            repo_root,
            deps_root,
            dialect: AxlScriptEvaluator::dialect(),
            globals: AxlScriptEvaluator::globals(),
            async_runtime: AsyncRuntime::new(),
        }
    }

    /// Evaluates the given .axl script path relative to the repository root, returning
    /// the evaluated script or an error. Performs security checks to ensure the script
    /// file is within the repository and isn't in a module directory.
    pub fn eval(&self, load_path: &Path) -> Result<EvaluatedAxlScript, EvalError> {
        let (module_name, load_path) = sanitize_load_path_lexically(load_path.to_str().unwrap())?;

        // Don't allow evaluating scripts directly from modules (e.g., via @module/ paths)
        if module_name.is_some() {
            return Err(EvalError::UnknownError(anyhow::anyhow!(
                "AXL scripts cannot be loaded directly from a module (load path starts with '@'): {}",
                load_path.display(),
            )));
        }

        // Ensure that we're not evaluating a script outside of the repository root
        let abs_script_path = normalize_abs_path_lexically(&self.repo_root.join(load_path))?;
        if !abs_script_path.starts_with(&self.repo_root) {
            return Err(EvalError::UnknownError(anyhow::anyhow!(
                "AXL script path {} resolves outside the repository root {}",
                abs_script_path.display(),
                self.repo_root.display()
            )));
        }

        // Create an AxlLoader to handle load statement within the script during evaluation
        let loader = AxlLoader {
            script_evaluator: self,
            script_dir: abs_script_path
                .parent()
                .expect("file path has parent")
                .to_path_buf(),
            root_dir: self.repo_root.clone(),
            root_deps_dir: self.deps_root.clone(),
        };

        // Push the script path onto the LOAD_STACK (used to detect circular loads)
        LOAD_STACK.with(|stack| stack.borrow_mut().push(abs_script_path.clone()));

        // Load and evaluate the script
        let raw = fs::read_to_string(&abs_script_path)?;
        let ast = AstModule::parse(&abs_script_path.to_string_lossy(), raw, &self.dialect)?;
        let module = Module::new();
        let mut eval = Evaluator::new(&module);
        eval.set_loader(&loader);
        eval.extra = Some(&self.async_runtime);
        eval.eval_module(ast, &self.globals)?;
        drop(eval);

        // Pop the script path off of the LOAD_STACK
        LOAD_STACK.with(|stack| stack.borrow_mut().pop());

        // Return the evaluated script
        Ok(EvaluatedAxlScript::new(
            abs_script_path,
            self.async_runtime.clone(),
            module,
        ))
    }
}
