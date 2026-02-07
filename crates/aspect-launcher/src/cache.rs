use std::fs;
use std::path::PathBuf;

use dirs::cache_dir;
use miette::{Context, IntoDiagnostic, Result, miette};
use sha2::{Digest, Sha256};

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

        let aspect_data_dir = data_dir.join(PathBuf::from("aspect/launcher"));
        fs::create_dir_all(&aspect_data_dir)
            .into_diagnostic()
            .wrap_err("unable to create `aspect` cache dir")?;

        Ok(AspectCache::from(aspect_data_dir))
    }

    pub fn tool_path(&self, tool_name: &String, tool_source: &String) -> PathBuf {
        let mut hasher = Sha256::new();
        hasher.update(tool_name.as_bytes());
        hasher.update(tool_source.as_bytes());
        let hash = format!("{:x}", hasher.finalize());
        self.root.join(format!("bin/{0}/{1}/{0}", tool_name, hash))
    }
}
