use anyhow::Result;
use ssri::Integrity;
use std::cell::RefCell;
use std::collections::HashMap;
use std::path::PathBuf;
use std::rc::Rc;

use allocative::Allocative;
use derive_more::Display;
use starlark::eval::Evaluator;
use starlark::starlark_simple_value;
use starlark::values::NoSerialize;
use starlark::values::ProvidesStaticType;
use starlark::values::StarlarkValue;
use starlark::values::starlark_value;

#[derive(Clone, Debug)]
pub struct UseConfigEntry {
    pub path: String,
    pub function: String,
    pub requires: Vec<(String, Option<String>)>,
    pub conflicts: Vec<String>,
}

#[derive(Debug, ProvidesStaticType, Default)]
pub struct ModuleStore {
    pub root_dir: PathBuf,
    pub module_name: String,
    pub module_root: PathBuf,
    pub deps: Rc<RefCell<HashMap<String, Dep>>>,
    pub tasks: Rc<RefCell<HashMap<PathBuf, (String, Vec<String>)>>>,
    pub configs: Rc<RefCell<Vec<UseConfigEntry>>>,
}

impl ModuleStore {
    pub fn new(root_dir: PathBuf, module_name: String, module_root: PathBuf) -> Self {
        Self {
            root_dir,
            module_name,
            module_root,
            deps: Rc::new(RefCell::new(HashMap::new())),
            tasks: Rc::new(RefCell::new(HashMap::new())),
            configs: Rc::new(RefCell::new(Vec::new())),
        }
    }

    pub fn from_eval<'v>(eval: &mut Evaluator<'v, '_, '_>) -> Result<ModuleStore> {
        let value = eval
            .extra
            .ok_or(anyhow::anyhow!("failed to get module store"))?
            .downcast_ref::<ModuleStore>()
            .ok_or(anyhow::anyhow!("failed to cast module store"))?;
        Ok(ModuleStore {
            root_dir: value.root_dir.clone(),
            module_name: value.module_name.clone(),
            module_root: value.module_root.clone(),
            deps: Rc::clone(&value.deps),
            tasks: Rc::clone(&value.tasks),
            configs: Rc::clone(&value.configs),
        })
    }
}

#[derive(Clone, Debug, ProvidesStaticType, Allocative)]
pub enum Dep {
    Local(AxlLocalDep),
    Remote(AxlArchiveDep),
}

impl Dep {
    pub fn name(&self) -> &String {
        match self {
            Dep::Local(local) => &local.name,
            Dep::Remote(remote) => &remote.name,
        }
    }

    pub fn use_config(&self) -> bool {
        match self {
            Dep::Local(local) => local.use_config,
            Dep::Remote(remote) => remote.use_config,
        }
    }
}

#[derive(Clone, Debug, ProvidesStaticType, NoSerialize, Allocative, Display)]
#[display("AxlLocalDep")]
pub struct AxlLocalDep {
    pub name: String,
    pub path: PathBuf,
    pub auto_use_tasks: bool,
    pub use_config: bool,
}

#[starlark_value(type = "AxlLocalDep")]
impl<'v> StarlarkValue<'v> for AxlLocalDep {}

starlark_simple_value!(AxlLocalDep);

#[derive(Clone, Debug, ProvidesStaticType, NoSerialize, Allocative, Display)]
#[display("AxlArchiveDep")]
pub struct AxlArchiveDep {
    pub urls: Vec<String>,
    #[allocative(skip)]
    pub integrity: Option<Integrity>,
    pub dev: bool,
    pub name: String,
    pub strip_prefix: String,
    pub auto_use_tasks: bool,
    pub use_config: bool,
}

#[starlark_value(type = "AxlArchiveDep")]
impl<'v> StarlarkValue<'v> for AxlArchiveDep {}

starlark_simple_value!(AxlArchiveDep);
