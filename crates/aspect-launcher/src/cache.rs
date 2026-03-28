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
        let aspect_data_dir = match std::env::var("ASPECT_CLI_DOWNLOADER_CACHE") {
            Ok(val) if !val.is_empty() => PathBuf::from(val).join("launcher"),
            _ => {
                let Some(data_dir) = cache_dir() else {
                    return Err(miette!("unable to identify the user's cache directory"));
                };
                data_dir.join(PathBuf::from("aspect/launcher"))
            }
        };
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_tool_path_structure() {
        let cache = AspectCache::from(PathBuf::from("/tmp/cache"));
        let path = cache.tool_path(
            &"aspect-cli".to_string(),
            &"https://github.com/aspect-build/aspect-cli/releases/tags/v1.0.0".to_string(),
        );
        // Path should be: /tmp/cache/bin/{tool_name}/{hash}/{tool_name}
        let components: Vec<_> = path.components().collect();
        let path_str = path.to_str().unwrap();
        assert!(path_str.starts_with("/tmp/cache/bin/aspect-cli/"));
        assert!(path_str.ends_with("/aspect-cli"));
        // Should have the structure: root/bin/name/hash/name
        assert_eq!(components.len(), 7); // /tmp/cache/bin/aspect-cli/{hash}/aspect-cli
    }

    #[test]
    fn test_tool_path_deterministic() {
        let cache = AspectCache::from(PathBuf::from("/tmp/cache"));
        let path1 = cache.tool_path(&"tool".to_string(), &"source-a".to_string());
        let path2 = cache.tool_path(&"tool".to_string(), &"source-a".to_string());
        assert_eq!(path1, path2);
    }

    #[test]
    fn test_tool_path_different_sources_differ() {
        let cache = AspectCache::from(PathBuf::from("/tmp/cache"));
        let path1 = cache.tool_path(&"tool".to_string(), &"source-a".to_string());
        let path2 = cache.tool_path(&"tool".to_string(), &"source-b".to_string());
        assert_ne!(path1, path2);
    }

    #[test]
    fn test_tool_path_different_names_differ() {
        let cache = AspectCache::from(PathBuf::from("/tmp/cache"));
        let path1 = cache.tool_path(&"tool-a".to_string(), &"source".to_string());
        let path2 = cache.tool_path(&"tool-b".to_string(), &"source".to_string());
        assert_ne!(path1, path2);
    }
}
