mod load;

use std::cell::RefCell;
use std::fs;
use std::path::{Path, PathBuf};
use std::rc::Rc;

use anyhow::anyhow;
use starlark::environment::{Globals, GlobalsBuilder, LibraryExtension, Module};
use starlark::eval::Evaluator;
use starlark::syntax::{AstModule, Dialect, DialectTypes};
use starlark::values::{Heap, ValueLike};
use thiserror::Error;

use crate::engine::config_context::ConfigContext;
use crate::engine::store::AxlStore;
use crate::engine::task::{AsTaskLike, FrozenTask, TaskLike};
use crate::engine::task_args::TaskArgs;
use crate::engine::task_context::TaskContext;
use crate::engine::{self, task::Task};
use crate::eval::load::AxlLoader;
use crate::helpers::{normalize_abs_path_lexically, sanitize_load_path_lexically, LoadPath};
use crate::module::AXL_ROOT_MODULE_NAME;

/// The core evaluator for .axl files, holding configuration like module root,
/// Starlark dialect, globals, and store. Used to evaluate .axl files securely.
#[derive(Debug)]
pub struct AxlScriptEvaluator {
    module_name: String,
    module_root: PathBuf,
    axl_deps_root: PathBuf,
    dialect: Dialect,
    globals: Globals,
    store: AxlStore,
}

/// Represents the result of evaluating an .axl script, encapsulating the module,
/// path, and store for task definition retrieval and execution.
#[derive(Debug, Clone)]
pub struct EvaluatedAxlScript {
    pub script_path: PathBuf,
    pub module_name: String,
    pub module_subpath: String,
    pub module: Rc<Module>,
    store: AxlStore,
}

/// Enum representing possible errors during evaluation, including Starlark-specific errors,
/// missing symbols, and wrapped anyhow or IO errors.
#[derive(Error, Debug)]
#[non_exhaustive]
pub enum EvalError {
    #[error("{0}")]
    StarlarkError(starlark::Error),

    #[error("axl script {0:?} does not export {1:?} symbol")]
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
    engine::register_globals(&mut globals);
    globals
}

// Custom From implementation since starlark::Error doesn't implement std::error::Error.
impl From<starlark::Error> for EvalError {
    fn from(value: starlark::Error) -> Self {
        Self::StarlarkError(value)
    }
}

impl EvaluatedAxlScript {
    fn new(
        script_path: PathBuf,
        module_name: String,
        module_subpath: String,
        store: AxlStore,
        module: Module,
    ) -> Self {
        Self {
            script_path,
            module_name,
            module_subpath,
            module: Rc::new(module),
            store,
        }
    }

    /// Retrieves a task definition from the evaluated module by symbol name.
    pub fn task_definition(&self, symbol: &str) -> Result<&dyn TaskLike, EvalError> {
        let def = self.module.get(symbol).ok_or(EvalError::MissingSymbol(
            self.script_path.clone(),
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
    pub fn execute_task(
        &self,
        symbol: &str,
        args: impl FnOnce(&Heap) -> TaskArgs,
    ) -> Result<Option<u8>, EvalError> {
        let def = self.module.get(symbol).ok_or(EvalError::MissingSymbol(
            self.script_path.clone(),
            symbol.to_string(),
        ))?;

        let heap = self.module.heap();
        let args = args(heap);
        let context = heap.alloc(TaskContext::new(args));
        let mut eval = Evaluator::new(&self.module);
        eval.extra = Some(&self.store);
        let ret = if let Some(val) = def.downcast_ref::<Task>() {
            eval.eval_function(val.implementation(), &[context], &[])?
        } else if let Some(val) = def.downcast_ref::<FrozenTask>() {
            eval.eval_function(val.implementation().to_value(), &[context], &[])?
        } else {
            return Err(EvalError::UnknownError(anyhow::anyhow!(
                "expected value of type Task"
            )));
        };
        Ok(ret.unpack_i32().map(|ex| ex as u8))
    }

    /// Executes a config function from the module by symbol, providing
    /// the config context.
    pub fn execute_config(&self, symbol: &str) -> Result<(), EvalError> {
        let def = self.module.get(symbol).ok_or(EvalError::MissingSymbol(
            self.script_path.clone(),
            symbol.to_string(),
        ))?;

        let heap = self.module.heap();
        let context = heap.alloc(ConfigContext::new());
        let mut eval = Evaluator::new(&self.module);
        eval.extra = Some(&self.store);
        eval.eval_function(def, &[context], &[])?;
        Ok(())
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

    /// Creates a new AxlScriptEvaluator with the given module root.
    pub fn new(
        module_name: String,
        module_root: PathBuf,
        axl_deps_root: PathBuf,
        root_dir: PathBuf,
    ) -> Self {
        Self {
            module_name,
            module_root,
            axl_deps_root,
            dialect: AxlScriptEvaluator::dialect(),
            globals: AxlScriptEvaluator::globals(),
            store: AxlStore::new(&root_dir),
        }
    }

    /// Evaluates the given .axl script path relative to the module root, returning
    /// the evaluated script or an error. Performs security checks to ensure the script
    /// file is within the module root.
    pub fn eval(&self, script_path: &Path) -> Result<EvaluatedAxlScript, EvalError> {
        let script_path = sanitize_load_path_lexically(script_path.to_str().unwrap())?;

        let script_subpath = match script_path {
            LoadPath::ModuleSpecifier { module, subpath } => {
                return Err(EvalError::UnknownError(anyhow::anyhow!(
                    "axl scripts cannot be loaded directly from a module (load path starts with '@'): @{}//{}",
                    module,
                    subpath.display(),
                )));
            }
            LoadPath::ModuleSubpath(subpath) | LoadPath::RelativePath(subpath) => subpath,
        };

        // Ensure that we're not evaluating a script outside of the module root
        let abs_script_path =
            normalize_abs_path_lexically(&self.module_root.join(&script_subpath))?;
        if !abs_script_path.starts_with(&self.module_root) {
            return Err(EvalError::UnknownError(anyhow::anyhow!(
                "axl script path {} resolves outside the module root {}",
                abs_script_path.display(),
                self.module_root.display()
            )));
        }

        // Create an AxlLoader to handle load statement within the script during evaluation
        let loader = AxlLoader {
            script_evaluator: self,
            script_dir: abs_script_path
                .parent()
                .expect("file path has parent")
                .to_path_buf(),
            module_name: AXL_ROOT_MODULE_NAME.to_string(),
            module_root: self.module_root.clone(),
            axl_deps_root: self.axl_deps_root.clone(),
        };

        // Push the script path onto the LOAD_STACK (used to detect circular loads)
        LOAD_STACK.with(|stack| stack.borrow_mut().push(abs_script_path.clone()));

        // Load and evaluate the script
        let raw = fs::read_to_string(&abs_script_path)?;
        let ast = AstModule::parse(&abs_script_path.to_string_lossy(), raw, &self.dialect)?;
        let module = Module::new();
        let mut eval = Evaluator::new(&module);
        eval.set_loader(&loader);
        eval.extra = Some(&self.store);
        eval.eval_module(ast, &self.globals)?;
        drop(eval);

        // Pop the script path off of the LOAD_STACK
        LOAD_STACK.with(|stack| stack.borrow_mut().pop());

        // Return the evaluated script
        Ok(EvaluatedAxlScript::new(
            abs_script_path,
            self.module_name.clone(),
            script_subpath.display().to_string(),
            self.store.clone(),
            module,
        ))
    }
}
