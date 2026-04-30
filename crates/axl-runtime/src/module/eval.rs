use std::fs;
use std::path::{Path, PathBuf};

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

use super::super::eval::{EvalError, join_confined, validate_module_name};

use super::module::{AxlArchiveDep, Mod};

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
        #[starlark(require = named, default = false)]
        auto_use_tasks: bool,
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
        validate_module_name(&name)?;

        if !dev {
            anyhow::bail!("axl_archive_dep does not support transitive dependencies yet.");
        }

        for url in &urls.items {
            if !url.ends_with(".tar.gz") {
                anyhow::bail!("only .tar.gz archives are supported at the moment.")
            }
        }

        let store = Mod::from_eval(eval)?;

        let integrity = if integrity.is_empty() {
            None
        } else {
            Some(integrity.parse()?)
        };

        let success = store.deps.insert(Dep::Remote(AxlArchiveDep {
            name: name.to_owned(),
            strip_prefix,
            integrity,
            auto_use_tasks,
            urls: urls.items,
            dev: true,
        }));

        if !success {
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
        validate_module_name(&name)?;

        let store = Mod::from_eval(eval)?;

        let mut abs_path = PathBuf::from(path);
        if !abs_path.is_absolute() {
            abs_path = store.root_dir.join(&abs_path).canonicalize()?;
        }

        let metadata = abs_path
            .metadata()
            .context(format!("failed to stat the path {:?}", abs_path))?;

        if !metadata.is_dir() {
            anyhow::bail!("path `{:?}` is not a directory", abs_path);
        }

        let success = store.deps.insert(Dep::Local(AxlLocalDep {
            name: name.to_owned(),
            path: abs_path,
            auto_use_tasks,
        }));

        if !success {
            anyhow::bail!("duplicate axl dep `{}` was declared previously.", name)
        }

        Ok(values::none::NoneType)
    }

    fn use_task<'v>(
        #[starlark(require = pos)] label: String,
        #[starlark(args)] symbols: UnpackListOrTuple<String>,
        eval: &mut Evaluator<'v, '_, '_>,
    ) -> anyhow::Result<values::none::NoneType> {
        let store = Mod::from_eval(eval)?;
        let absolute_path = join_confined(&store.root, Path::new(&label))?;

        let entry = store.tasks.entry(absolute_path).or_insert((label, vec![]));
        entry.1.extend(symbols);

        Ok(values::none::NoneType)
    }

    /// Declares a feature to load and make globally available.
    ///
    /// The feature's `implementation` function will run after all config.axl files
    /// have been evaluated, injecting behavior into fragment hook lists.
    ///
    /// Example:
    /// ```
    /// use_feature("@aspect//feature/artifacts.axl", "ArtifactUpload")
    /// ```
    fn use_feature<'v>(
        #[starlark(require = pos)] label: String,
        #[starlark(require = pos)] symbol: String,
        eval: &mut Evaluator<'v, '_, '_>,
    ) -> anyhow::Result<values::none::NoneType> {
        let r#mod = Mod::from_eval(eval)?;
        let absolute_path = join_confined(&r#mod.root, Path::new(&label))?;
        r#mod.features.push((absolute_path, symbol));
        Ok(values::none::NoneType)
    }
}

pub const AXL_MODULE_FILE: &str = "MODULE.aspect";

pub const AXL_ROOT_MODULE_NAME: &str = "root";

pub const AXL_SCRIPT_EXTENSION: &str = "axl";

pub const AXL_CONFIG_EXTENSION: &str = "config.axl";

pub const AXL_VERSION_EXTENSION: &str = "version.axl";

/// Returns a GlobalsBuilder for MODULE.aspect specific AXL globals, extending
/// various Starlark library extensions with custom top-level functions.
pub fn get_globals() -> Globals {
    let globals = GlobalsBuilder::extended_by(&[
        LibraryExtension::Breakpoint,
        LibraryExtension::CallStack,
        LibraryExtension::Debug,
        LibraryExtension::EnumType,
        LibraryExtension::Filter,
        // NB: `LibraryExtension::Json` is intentionally skipped — we
        // register a custom `json` namespace below that adds `try_decode`
        // alongside `encode`/`decode`. See engine::builtins::register_json.
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
    globals
        .with(crate::engine::builtins::register_json)
        .with(register_globals)
        .build()
}

/// Evaluator for MODULE.aspect
#[derive(Debug)]
pub struct ModEvaluator {
    root_dir: PathBuf,
    dialect: Dialect,
    globals: Globals,
}

impl ModEvaluator {
    pub fn new(root_dir: PathBuf) -> Self {
        Self {
            root_dir,
            dialect: ModEvaluator::dialect(),
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

    pub fn evaluate(&self, name: String, root: PathBuf) -> Result<Mod, EvalError> {
        let is_root_module = name == AXL_ROOT_MODULE_NAME;
        let axl_filename = if is_root_module {
            AXL_MODULE_FILE.to_string()
        } else {
            format!("{}/{}", root.to_str().unwrap(), AXL_MODULE_FILE)
        };

        let module_boundary_path = &root.join(AXL_MODULE_FILE);

        let mut r#mod = Mod::new(self.root_dir.to_path_buf(), name, root);

        if module_boundary_path.exists() {
            let contents = fs::read_to_string(module_boundary_path)?;
            let ast = AstModule::parse(&axl_filename, contents, &self.dialect)?;
            Module::with_temp_heap(|module| {
                let mut eval = Evaluator::new(&module);
                eval.extra_mut = Some(&mut r#mod);
                eval.eval_module(ast, &self.globals)?;
                Ok::<_, starlark::Error>(())
            })?;
        }

        Ok(r#mod)
    }
}
