//! `config.json`: the user's `~/.aspect/config.json` and a repository's
//! `.aspect/config.json`, read as JSON.
//!
//! Both files hold `deployments` (see `crate::engine::aspect::auth`, which
//! merges the two). Only the user's file holds `editor`: what a link to a file
//! opens ([`editor`]), a personal choice a repository does not make.

use std::path::{Path, PathBuf};

/// `~/.aspect/config.json`.
pub(crate) fn user_path() -> anyhow::Result<PathBuf> {
    let home =
        dirs::home_dir().ok_or_else(|| anyhow::anyhow!("unable to determine home directory"))?;
    Ok(home.join(".aspect").join("config.json"))
}

/// The text of the `config.json` at `path`, `None` when there is no such file.
pub(crate) fn read(path: &Path) -> anyhow::Result<Option<String>> {
    match std::fs::read_to_string(path) {
        Ok(content) => Ok(Some(content)),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(e) => Err(anyhow::anyhow!("failed to read {}: {}", path.display(), e)),
    }
}

/// The top-level `editor` string of the `config.json` at `path`: `None` when
/// the file or the key is missing, an error naming the file and key when the
/// file cannot be read or parsed or the value is not a string.
pub(crate) fn editor(path: &Path) -> Result<Option<String>, String> {
    let Some(content) = read(path).map_err(|e| format!("\"editor\": {e}"))? else {
        return Ok(None);
    };
    let json: serde_json::Value = serde_json::from_str(&content)
        .map_err(|e| format!("failed to parse {} for \"editor\": {e}", path.display()))?;
    match json.get("editor") {
        None => Ok(None),
        Some(serde_json::Value::String(value)) => Ok(Some(value.clone())),
        Some(other) => Err(format!(
            "{} \"editor\" must be a string, got {other}",
            path.display()
        )),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn editor_reads_the_key_and_reports_a_missing_file_or_key_as_none() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.json");
        assert_eq!(editor(&path), Ok(None), "missing file");
        std::fs::write(&path, r#"{"deployments": []}"#).unwrap();
        assert_eq!(editor(&path), Ok(None), "missing key");
        std::fs::write(&path, r#"{"editor": "zed", "deployments": []}"#).unwrap();
        assert_eq!(editor(&path), Ok(Some("zed".into())));
    }

    #[test]
    fn editor_refuses_a_non_string_or_malformed_file() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.json");
        std::fs::write(&path, r#"{"editor": 1}"#).unwrap();
        let err = editor(&path).unwrap_err();
        assert!(err.contains("\"editor\" must be a string"), "{err}");
        assert!(err.contains(&path.display().to_string()), "{err}");
        std::fs::write(&path, "{").unwrap();
        let err = editor(&path).unwrap_err();
        assert!(err.starts_with("failed to parse"), "{err}");
        assert!(err.contains("\"editor\""), "{err}");
    }
}
