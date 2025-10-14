use std::path::PathBuf;

use anyhow::anyhow;
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

use crate::module::store::Override;

use super::super::eval::EvalError;
use super::store::{AxlDep, ModuleStore};

#[starlark_module]
pub fn register_toplevels(_: &mut GlobalsBuilder) {
    fn axl_dep<'v>(
        #[starlark(require = named)] name: String,
        #[starlark(require = named)] integrity: String,
        #[starlark(require = named)] urls: UnpackList<String>,
        #[starlark(require = named)] dev: bool,
        #[starlark(require = named, default = String::new())] strip_prefix: String,
        eval: &mut Evaluator<'v, '_, '_>,
    ) -> anyhow::Result<values::none::NoneType> {
        if !dev {
            anyhow::bail!("axl_dep does not support transitive dependencies yet.");
        }
        for url in &urls.items {
            if !url.ends_with(".tar.gz") {
                anyhow::bail!("only .tar.gz archives are supported at the moment.")
            }
        }
        let store = ModuleStore::from_eval(eval)?;
        let prev_dep = store.deps.borrow_mut().insert(
            name.clone(),
            AxlDep {
                urls: urls.items,
                name,
                integrity: integrity.parse()?,
                strip_prefix,
                dev: true,
                r#override: None,
            },
        );
        if prev_dep.is_some() {
            anyhow::bail!(
                "duplicate axl_dep `{}` was declared previously.",
                prev_dep.unwrap().name
            )
        }
        Ok(values::none::NoneType)
    }

    fn local_path_override<'v>(
        #[starlark(require = named)] dep_name: String,
        #[starlark(require = named)] path: String,
        eval: &mut Evaluator<'v, '_, '_>,
    ) -> anyhow::Result<values::none::NoneType> {
        let store = ModuleStore::from_eval(eval)?;
        let mut deps = store.deps.borrow_mut();
        let dep = deps
            .get_mut(&dep_name)
            .ok_or(anyhow!("axl_dep `{}` is not declared.", dep_name))?;
        if dep.r#override.is_some() {
            anyhow::bail!("axl_dep `{}` already has an override.", dep_name);
        }

        let abs_path = store.repo_root.join(&path).canonicalize()?;
        let metadata = abs_path
            .metadata()
            .context(format!("failed to stat the path {:?}", abs_path))?;

        if !metadata.is_dir() {
            anyhow::bail!("path `{}` is not a directory", &path);
        }

        dep.r#override = Some(Override::Local { path: abs_path });

        Ok(values::none::NoneType)
    }
}

pub const BOUNDARY_FILE: &str = "MODULE.aspect";

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
    globals.with(register_toplevels).build()
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

    pub fn evaluate(&self, name: &str, script: &str) -> Result<ModuleStore, EvalError> {
        let ast = AstModule::parse(name, script.to_string(), &self.dialect)?;

        let extension_store = ModuleStore::new(self.repo_root.clone());
        {
            let module = Module::new();
            let mut eval = Evaluator::new(&module);
            eval.extra = Some(&extension_store);
            eval.eval_module(ast, &self.globals)?;
        }
        Ok(extension_store)
    }
}
