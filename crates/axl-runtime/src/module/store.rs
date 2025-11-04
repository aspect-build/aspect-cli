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
use starlark::values::starlark_value;
use starlark::values::NoSerialize;
use starlark::values::ProvidesStaticType;
use starlark::values::StarlarkValue;

#[derive(Debug, ProvidesStaticType, Default)]
pub struct ModuleStore {
    pub repo_root: PathBuf,
    pub module_name: String,
    pub module_root: PathBuf,
    pub deps: Rc<RefCell<HashMap<String, Dep>>>,
    pub tasks: Rc<RefCell<Vec<(String, String)>>>,
}

impl ModuleStore {
    pub fn new(repo_root: PathBuf, module_name: String, module_root: PathBuf) -> Self {
        Self {
            repo_root,
            module_name,
            module_root,
            deps: Rc::new(RefCell::new(HashMap::new())),
            tasks: Rc::new(RefCell::new(Vec::new())),
        }
    }

    pub fn from_eval<'v>(eval: &mut Evaluator<'v, '_, '_>) -> Result<ModuleStore> {
        let value = eval
            .extra
            .ok_or(anyhow::anyhow!("failed to get module store"))?
            .downcast_ref::<ModuleStore>()
            .ok_or(anyhow::anyhow!("failed to cast module store"))?;
        Ok(ModuleStore {
            repo_root: value.repo_root.clone(),
            module_name: value.module_name.clone(),
            module_root: value.module_root.clone(),
            deps: Rc::clone(&value.deps),
            tasks: Rc::clone(&value.tasks),
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
}

#[derive(Clone, Debug, ProvidesStaticType, NoSerialize, Allocative, Display)]
#[display("axl_local_dep")]
pub struct AxlLocalDep {
    pub name: String,
    pub path: PathBuf,
    pub autouse: bool,
}

#[starlark_value(type = "axl_local_dep")]
impl<'v> StarlarkValue<'v> for AxlLocalDep {}

starlark_simple_value!(AxlLocalDep);

#[derive(Clone, Debug, ProvidesStaticType, NoSerialize, Allocative, Display)]
#[display("axl_archive_dep")]
pub struct AxlArchiveDep {
    pub urls: Vec<String>,
    #[allocative(skip)]
    pub integrity: Integrity,
    pub dev: bool,
    pub name: String,
    pub strip_prefix: String,
    pub autouse: bool,
}

#[starlark_value(type = "axl_archive_dep")]
impl<'v> StarlarkValue<'v> for AxlArchiveDep {}

starlark_simple_value!(AxlArchiveDep);
