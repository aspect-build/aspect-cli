use std::fs;
use std::path::PathBuf;

use anyhow::Context;
use anyhow::Result;
use starlark::environment::Globals;
use starlark::environment::GlobalsBuilder;
use starlark::environment::LibraryExtension;
use starlark::environment::Module;
use starlark::eval::Evaluator;
use starlark::starlark_module;
use starlark::syntax::AstModule;
use starlark::syntax::Dialect;
use starlark::syntax::DialectTypes;
use starlark::values;
use starlark::values::list::UnpackList;
use starlark::values::list_or_tuple::UnpackListOrTuple;

use crate::module::AxlLocalDep;
use crate::module::Dep;

use super::super::eval::EvalError;
use super::super::helpers::validate_module_name;
use super::store::{AxlArchiveDep, ModuleStore};

#[starlark_module]
pub fn register_globals(globals: &mut GlobalsBuilder) {
    fn axl_dep<'v>(
        #[allow(unused)]
        #[starlark(require = named)]
        name: String,
        #[allow(unused)]
        #[starlark(require = named)]
        integrity: String,
        #[allow(unused)]
        #[starlark(require = named)]
        urls: UnpackList<String>,
        #[allow(unused)]
        #[starlark(require = named)]
        dev: bool,
        #[allow(unused)]
        #[starlark(require = named, default = String::new())]
        strip_prefix: String,
    ) -> anyhow::Result<values::none::NoneType> {
        Err(anyhow::anyhow!(
            "axl_dep has been renamed to axl_archive_dep"
        ))
    }

    fn local_path_override<'v>(
        #[allow(unused)]
        #[starlark(require = named)]
        name: String,
        #[allow(unused)]
        #[starlark(require = named)]
        path: String,
    ) -> anyhow::Result<values::none::NoneType> {
        Err(anyhow::anyhow!(
            "local_path_override has been renamed to axl_local_dep"
        ))
    }

    fn axl_archive_dep<'v>(
        #[starlark(require = named)] name: String,
        #[starlark(require = named, default = String::new())] integrity: String, // allow unset integrity; it is required but we'll error out later with a more helpful error message
        #[starlark(require = named)] urls: UnpackList<String>,
        #[starlark(require = named)] dev: bool,
        #[starlark(require = named, default = false)] auto_use_tasks: bool,
        #[starlark(require = named, default = String::new())] strip_prefix: String,
        eval: &mut Evaluator<'v, '_, '_>,
    ) -> anyhow::Result<values::none::NoneType> {
        if name == AXL_ROOT_MODULE_NAME {
            anyhow::bail!(
                "axl_archive_dep name {:?} not allowed.",
                AXL_ROOT_MODULE_NAME
            );
        }
        validate_module_name(&name).map_err(|e| e.into_anyhow())?;

        if !dev {
            anyhow::bail!("axl_archive_dep does not support transitive dependencies yet.");
        }

        for url in &urls.items {
            if !url.ends_with(".tar.gz") {
                anyhow::bail!("only .tar.gz archives are supported at the moment.")
            }
        }

        let store = ModuleStore::from_eval(eval)?;

        let integrity = if integrity.is_empty() {
            None
        } else {
            Some(integrity.parse()?)
        };

        let prev_dep = store.deps.borrow_mut().insert(
            name.clone(),
            Dep::Remote(AxlArchiveDep {
                name: name.clone(),
                strip_prefix,
                urls: urls.items,
                integrity,
                dev: true,
                auto_use_tasks,
            }),
        );

        if prev_dep.is_some() {
            anyhow::bail!("duplicate axl dep `{}` was declared previously.", name)
        }

        Ok(values::none::NoneType)
    }

    fn axl_local_dep<'v>(
        #[starlark(require = named)] name: String,
        #[starlark(require = named)] path: String,
        #[starlark(require = named, default = false)] auto_use_tasks: bool,
        eval: &mut Evaluator<'v, '_, '_>,
    ) -> anyhow::Result<values::none::NoneType> {
        if name == AXL_ROOT_MODULE_NAME {
            anyhow::bail!("axl_local_dep name {:?} not allowed.", AXL_ROOT_MODULE_NAME);
        }
        validate_module_name(&name).map_err(|e| e.into_anyhow())?;

        let store = ModuleStore::from_eval(eval)?;

        let mut abs_path = PathBuf::from(path);
        if !abs_path.is_absolute() {
            abs_path = store.repo_root.join(&abs_path).canonicalize()?;
        }

        let metadata = abs_path
            .metadata()
            .context(format!("failed to stat the path {:?}", abs_path))?;

        if !metadata.is_dir() {
            anyhow::bail!("path `{:?}` is not a directory", abs_path);
        }

        let prev_dep = store.deps.borrow_mut().insert(
            name.clone(),
            Dep::Local(AxlLocalDep {
                name: name.clone(),
                path: abs_path,
                auto_use_tasks,
            }),
        );

        if prev_dep.is_some() {
            anyhow::bail!("duplicate axl dep `{}` was declared previously.", name)
        }

        Ok(values::none::NoneType)
    }

    fn use_task<'v>(
        #[starlark(require = pos)] label: String,
        #[starlark(args)] symbols: UnpackListOrTuple<String>,
        eval: &mut Evaluator<'v, '_, '_>,
    ) -> anyhow::Result<values::none::NoneType> {
        let store = ModuleStore::from_eval(eval)?;
        let mut task = store.tasks.borrow_mut();

        for symbol in symbols {
            task.push((label.clone(), symbol));
        }

        Ok(values::none::NoneType)
    }
}

pub const AXL_MODULE_FILE: &str = "MODULE.aspect";

pub const AXL_ROOT_MODULE_NAME: &str = "root";

#[allow(dead_code)]
pub const AXL_SCRIPT_EXTENSION: &str = "axl";

/// Returns a GlobalsBuilder for MODULE.aspect specific AXL globals, extending
/// various Starlark library extensions with custom top-level functions.
pub fn get_globals() -> Globals {
    let globals = GlobalsBuilder::extended_by(&[
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
    globals.with(register_globals).build()
}

/// Evaluator for MODULE.aspect
#[derive(Debug)]
pub struct AxlModuleEvaluator {
    repo_root: PathBuf,
    dialect: Dialect,
    globals: Globals,
}

impl AxlModuleEvaluator {
    pub fn new(repo_root: PathBuf) -> Self {
        Self {
            repo_root,
            dialect: AxlModuleEvaluator::dialect(),
            globals: get_globals(),
        }
    }

    /// Returns the configured Starlark dialect for MODULE.aspect files.
    fn dialect() -> Dialect {
        Dialect {
            enable_def: false,
            enable_lambda: false,
            enable_load: false,
            enable_load_reexport: false,
            enable_keyword_only_arguments: true,
            enable_positional_only_arguments: true,
            enable_types: DialectTypes::Enable,
            enable_f_strings: true,
            enable_top_level_stmt: true,
            ..Default::default()
        }
    }

    pub fn evaluate(
        &self,
        module_name: String,
        module_root: PathBuf,
    ) -> Result<ModuleStore, EvalError> {
        let is_root_module = module_name == AXL_ROOT_MODULE_NAME;
        let axl_filename = if is_root_module {
            AXL_MODULE_FILE.to_string()
        } else {
            module_root
                .join(AXL_MODULE_FILE)
                .to_string_lossy()
                .to_string()
        };

        let module_boundary_path = &module_root.join(AXL_MODULE_FILE);

        let store = ModuleStore::new(self.repo_root.to_path_buf(), module_name, module_root);

        if module_boundary_path.exists() {
            let contents = fs::read_to_string(module_boundary_path)?;

            let ast = AstModule::parse(&axl_filename, contents, &self.dialect)?;

            {
                let module = Module::new();
                let mut eval = Evaluator::new(&module);
                eval.extra = Some(&store);
                eval.eval_module(ast, &self.globals)?;
            }
        }

        Ok(store)
    }
}