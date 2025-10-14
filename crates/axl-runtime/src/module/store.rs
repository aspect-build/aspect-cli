use anyhow::Result;
use ssri::Integrity;
use std::cell::RefCell;
use std::collections::HashMap;
use std::fmt::Display;
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
    pub deps: Rc<RefCell<HashMap<String, AxlDep>>>,
}

impl ModuleStore {
    pub fn new(repo_root: PathBuf) -> Self {
        Self {
            repo_root,
            deps: Rc::new(RefCell::new(HashMap::new())),
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
            deps: Rc::clone(&value.deps),
        })
    }
}

#[derive(Clone, Debug, ProvidesStaticType, Allocative)]
pub enum Override {
    Local { path: PathBuf },
}

impl Display for Override {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Override::Local { path } => write!(f, "local_path_override(path={:?})", path),
        }
    }
}

#[derive(Clone, Debug, ProvidesStaticType, NoSerialize, Allocative, Display)]
#[display("axl_dep")]
pub struct AxlDep {
    pub urls: Vec<String>,
    #[allocative(skip)]
    pub integrity: Integrity,
    pub dev: bool,
    pub name: String,
    pub strip_prefix: String,
    pub r#override: Option<Override>,
}

#[starlark_value(type = "axl_dep")]
impl<'v> StarlarkValue<'v> for AxlDep {}

starlark_simple_value!(AxlDep);
