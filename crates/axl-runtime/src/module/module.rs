use anyhow::Result;
use ssri::Integrity;
use std::collections::HashMap;
use std::collections::HashSet;
use std::hash::Hash;
use std::path::PathBuf;

use allocative::Allocative;
use derive_more::Display;
use starlark::eval::Evaluator;
use starlark::starlark_simple_value;
use starlark::values::NoSerialize;
use starlark::values::ProvidesStaticType;
use starlark::values::StarlarkValue;
use starlark::values::starlark_value;

use crate::module::AXL_ROOT_MODULE_NAME;

#[derive(Debug, ProvidesStaticType, Default)]
pub struct Mod {
    pub root_dir: PathBuf,
    pub name: String,
    pub root: PathBuf,
    pub deps: HashSet<Dep>,
    pub tasks: HashMap<PathBuf, (String, Vec<String>)>,
    pub features: Vec<(PathBuf, String)>,
}

impl Mod {
    pub fn new(root_dir: PathBuf, name: String, root: PathBuf) -> Self {
        Self {
            root_dir,
            name,
            root,
            deps: HashSet::new(),
            tasks: HashMap::new(),
            features: Vec::new(),
        }
    }

    pub fn is_root(&self) -> bool {
        self.name == AXL_ROOT_MODULE_NAME
    }

    pub fn from_eval<'v, 'a, 'e>(eval: &'e mut Evaluator<'v, 'a, '_>) -> Result<&'e mut Mod> {
        let extra = eval
            .extra_mut
            .as_deref_mut()
            .ok_or_else(|| anyhow::anyhow!("failed to get module store"))?;
        extra
            .downcast_mut::<Mod>()
            .ok_or_else(|| anyhow::anyhow!("failed to cast module store"))
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

impl Eq for Dep {}

impl PartialEq for Dep {
    fn eq(&self, other: &Self) -> bool {
        self.name() == other.name()
    }
}

impl Hash for Dep {
    fn hash<H: std::hash::Hasher>(&self, state: &mut H) {
        self.name().hash(state);
    }
}

#[derive(Clone, Debug, ProvidesStaticType, NoSerialize, Allocative, Display)]
#[display("AxlLocalDep")]
pub struct AxlLocalDep {
    pub name: String,
    pub path: PathBuf,
    pub auto_use_tasks: bool,
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
}

#[starlark_value(type = "AxlArchiveDep")]
impl<'v> StarlarkValue<'v> for AxlArchiveDep {}

starlark_simple_value!(AxlArchiveDep);
