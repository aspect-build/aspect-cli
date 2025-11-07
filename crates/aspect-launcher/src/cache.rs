use std::path::PathBuf;

use aspect_config::ToolSpec;
use dirs::cache_dir;
use miette::{miette, Context, IntoDiagnostic, Result};
use std::fs;

#[derive(Debug, Clone)]
pub struct AspectCache {
    root: PathBuf,
}

impl AspectCache {
    pub fn from(root: PathBuf) -> AspectCache {
        AspectCache { root: root.clone() }
    }

    pub fn default() -> Result<AspectCache> {
        let Some(data_dir) = cache_dir() else {
            return Err(miette!("unable to identify the user's cache directory"));
        };

        let aspect_data_dir = data_dir.join(PathBuf::from("aspect"));
        fs::create_dir_all(&aspect_data_dir)
            .into_diagnostic()
            .wrap_err("unable to create `aspect` cache dir")?;

        Ok(AspectCache::from(aspect_data_dir))
    }

    pub fn tool_path(&self, tool: &dyn ToolSpec) -> PathBuf {
        self.root
            .join(format!("bin/{0}/{1}/{0}", tool.name(), tool.version()))
    }
}
