use std::fs;
use std::path::PathBuf;
use std::time::{Duration, SystemTime};

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

    /// Path to a small file that records the last resolved tag for an unpinned GitHub source.
    /// Keyed on (tool_name, org, repo, artifact) — no tag — so it can be read before any
    /// API call and used to reconstruct the binary cache path.
    pub fn latest_tag_path(
        &self,
        tool_name: &str,
        org: &str,
        repo: &str,
        artifact: &str,
    ) -> PathBuf {
        let mut hasher = Sha256::new();
        hasher.update(tool_name.as_bytes());
        hasher.update(org.as_bytes());
        hasher.update(repo.as_bytes());
        hasher.update(artifact.as_bytes());
        let hash = format!("{:x}", hasher.finalize());
        self.root.join(format!("latest/{}", hash))
    }

    /// Returns true if the tag hint file is present and was written within `max_age`.
    /// A stale or missing hint means the caller should re-query the releases API.
    pub fn latest_tag_is_fresh(&self, hint_path: &PathBuf, max_age: Duration) -> bool {
        match fs::metadata(hint_path) {
            Ok(meta) => match meta.modified() {
                Ok(mtime) => SystemTime::now()
                    .duration_since(mtime)
                    .map(|age| age < max_age)
                    .unwrap_or(false),
                Err(_) => false,
            },
            Err(_) => false,
        }
    }

    /// Touches the mtime of the hint file to the current time without changing its contents.
    /// Used to reset the expiry clock after a failed API call so we don't hammer a down API.
    pub fn touch_latest_tag(&self, hint_path: &PathBuf) {
        // Rewrite with the same content — simplest cross-platform mtime update.
        if let Ok(contents) = fs::read(hint_path) {
            let _ = fs::write(hint_path, contents);
        }
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
            &"https://github.com/aspect-build/aspect-cli/releases/download/v2026.15.2/aspect-cli-aarch64-apple-darwin".to_string(),
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

    #[test]
    fn test_latest_tag_path_structure() {
        let cache = AspectCache::from(PathBuf::from("/tmp/cache"));
        let path = cache.latest_tag_path(
            "aspect-cli",
            "aspect-build",
            "aspect-cli",
            "aspect-cli-aarch64-apple-darwin",
        );
        let path_str = path.to_str().unwrap();
        assert!(path_str.starts_with("/tmp/cache/latest/"));
        // No tool name in the filename — just the hash
        assert!(!path_str.ends_with("/aspect-cli"));
    }

    #[test]
    fn test_latest_tag_path_deterministic() {
        let cache = AspectCache::from(PathBuf::from("/tmp/cache"));
        let path1 = cache.latest_tag_path("aspect-cli", "aspect-build", "aspect-cli", "artifact");
        let path2 = cache.latest_tag_path("aspect-cli", "aspect-build", "aspect-cli", "artifact");
        assert_eq!(path1, path2);
    }

    #[test]
    fn test_latest_tag_path_different_artifacts_differ() {
        let cache = AspectCache::from(PathBuf::from("/tmp/cache"));
        let path1 = cache.latest_tag_path(
            "aspect-cli",
            "aspect-build",
            "aspect-cli",
            "artifact-darwin",
        );
        let path2 =
            cache.latest_tag_path("aspect-cli", "aspect-build", "aspect-cli", "artifact-linux");
        assert_ne!(path1, path2);
    }

    #[test]
    fn test_latest_tag_path_differs_from_tool_path() {
        let cache = AspectCache::from(PathBuf::from("/tmp/cache"));
        let tool_path = cache.tool_path(&"aspect-cli".to_string(), &"some-url".to_string());
        let hint_path =
            cache.latest_tag_path("aspect-cli", "aspect-build", "aspect-cli", "artifact");
        assert_ne!(tool_path, hint_path);
    }

    fn tmp_dir(label: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "aspect-cache-test-{}-{}",
            std::process::id(),
            label
        ));
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn test_latest_tag_is_fresh_when_just_written() {
        let tmp = tmp_dir("fresh");
        let cache = AspectCache::from(tmp.clone());
        let hint = cache.latest_tag_path("aspect-cli", "aspect-build", "aspect-cli", "artifact");
        fs::create_dir_all(hint.parent().unwrap()).unwrap();
        fs::write(&hint, "v2026.15.2").unwrap();

        assert!(cache.latest_tag_is_fresh(&hint, Duration::from_secs(3600)));
        fs::remove_dir_all(&tmp).unwrap();
    }

    #[test]
    fn test_latest_tag_is_stale_when_expired() {
        let tmp = tmp_dir("stale");
        let cache = AspectCache::from(tmp.clone());
        let hint = cache.latest_tag_path("aspect-cli", "aspect-build", "aspect-cli", "artifact");
        fs::create_dir_all(hint.parent().unwrap()).unwrap();
        fs::write(&hint, "v2026.15.2").unwrap();

        // Zero max-age means any file is immediately stale.
        assert!(!cache.latest_tag_is_fresh(&hint, Duration::ZERO));
        fs::remove_dir_all(&tmp).unwrap();
    }

    #[test]
    fn test_latest_tag_is_not_fresh_when_missing() {
        let tmp = tmp_dir("missing");
        let cache = AspectCache::from(tmp.clone());
        let hint = cache.latest_tag_path("aspect-cli", "aspect-build", "aspect-cli", "artifact");

        assert!(!cache.latest_tag_is_fresh(&hint, Duration::from_secs(3600)));
        fs::remove_dir_all(&tmp).unwrap();
    }

    #[test]
    fn test_touch_latest_tag_refreshes_mtime() {
        let tmp = tmp_dir("touch");
        let cache = AspectCache::from(tmp.clone());
        let hint = cache.latest_tag_path("aspect-cli", "aspect-build", "aspect-cli", "artifact");
        fs::create_dir_all(hint.parent().unwrap()).unwrap();
        fs::write(&hint, "v2026.15.2").unwrap();

        // After touching, the file should still be fresh and contents unchanged.
        cache.touch_latest_tag(&hint);
        assert!(cache.latest_tag_is_fresh(&hint, Duration::from_secs(3600)));
        assert_eq!(fs::read_to_string(&hint).unwrap(), "v2026.15.2");
        fs::remove_dir_all(&tmp).unwrap();
    }
}
