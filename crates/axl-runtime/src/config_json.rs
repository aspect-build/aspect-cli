//! `config.json`: the user's `~/.aspect/config.json` and a repository's
//! `.aspect/config.json`, read as JSON ([`object`]: an empty file is `{}`).
//!
//! Both files hold `deployments` (see `crate::engine::aspect::auth`, which
//! merges the two) and `editor`: what a link to a file opens ([`editor`]), the
//! repository's a team's default that the user's overrides (see
//! `crate::out::config_link_format`). Both hold `tips` too, unioned (see
//! `crate::user_config`, which also writes the user's file). A file that is
//! not a JSON object is one warning however many of its keys are read
//! ([`first_time`]).

use std::path::{Path, PathBuf};

/// `~/.aspect/config.json`.
pub(crate) fn user_path() -> anyhow::Result<PathBuf> {
    let home =
        dirs::home_dir().ok_or_else(|| anyhow::anyhow!("unable to determine home directory"))?;
    Ok(home.join(".aspect").join("config.json"))
}

/// A repository's `.aspect/config.json`: under the task's resolved root, else
/// the root found from the working directory; `None` outside a repository.
pub(crate) fn repo_path() -> Option<PathBuf> {
    let root = match crate::engine::store::resolved_aspect_root() {
        Some(root) => root.to_path_buf(),
        None => crate::project_root::aspect_root_from_cwd()?,
    };
    Some(
        root.join(crate::project_root::DOT_ASPECT_FOLDER)
            .join("config.json"),
    )
}

/// The text of the `config.json` at `path`, `None` when there is no such file.
pub(crate) fn read(path: &Path) -> anyhow::Result<Option<String>> {
    match std::fs::read_to_string(path) {
        Ok(content) => Ok(Some(content)),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(e) => Err(anyhow::anyhow!("failed to read {}: {}", path.display(), e)),
    }
}

/// The top-level object of the `config.json` at `path`: `None` when there is
/// no such file, empty for a file of only whitespace. Else why not, naming
/// the file and no key, so every reader of the file words it alike: it cannot
/// be read, is not JSON (saying so when it starts with a byte order mark or
/// holds a comment, which JSON allows neither), or is not an object.
pub(crate) fn object(
    path: &Path,
) -> Result<Option<serde_json::Map<String, serde_json::Value>>, String> {
    let Some(text) = read(path).map_err(|e| e.to_string())? else {
        return Ok(None);
    };
    if text.trim().is_empty() {
        return Ok(Some(serde_json::Map::new()));
    }
    match serde_json::from_str(&text) {
        Ok(serde_json::Value::Object(object)) => Ok(Some(object)),
        Ok(_) => Err(format!("{} is not a JSON object", path.display())),
        Err(e) => {
            let why = if text.starts_with('\u{feff}') {
                " (it starts with a byte order mark, which JSON does not allow)"
            } else if text.lines().any(|l| {
                let l = l.trim_start();
                l.starts_with("//") || l.starts_with("/*")
            }) {
                " (it holds a comment, which JSON does not allow)"
            } else {
                ""
            };
            Err(format!("failed to parse {}{why}: {e}", path.display()))
        }
    }
}

/// Whether `warning` is new to this process, so each shows once however
/// many readers meet it.
pub(crate) fn first_time(warning: &str) -> bool {
    static SEEN: std::sync::Mutex<Option<std::collections::HashSet<String>>> =
        std::sync::Mutex::new(None);
    let mut seen = SEEN.lock().unwrap_or_else(|e| e.into_inner());
    seen.get_or_insert_with(Default::default)
        .insert(warning.to_owned())
}

/// The top-level `editor` string of the `config.json` at `path`: `None` when
/// the file or the key is missing, else an error naming the file: the
/// file's own ([`object`]), or that the value is not a string.
pub(crate) fn editor(path: &Path) -> Result<Option<String>, String> {
    let Some(json) = object(path)? else {
        return Ok(None);
    };
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
        for root in ["[]", r#""zed""#, "null"] {
            std::fs::write(&path, root).unwrap();
            let err = editor(&path).unwrap_err();
            assert!(err.contains("is not a JSON object"), "{root}: {err}");
        }
    }

    #[test]
    fn object_reads_an_empty_file_as_empty_and_says_why_a_file_is_not_json() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.json");
        assert_eq!(object(&path), Ok(None), "missing file");
        for empty in ["", "  \n\t"] {
            std::fs::write(&path, empty).unwrap();
            assert_eq!(object(&path), Ok(Some(Default::default())), "{empty:?}");
        }
        std::fs::write(&path, "\u{feff}{}").unwrap();
        let err = object(&path).unwrap_err();
        assert!(err.contains("byte order mark"), "{err}");
        std::fs::write(&path, "{\n  // mine\n  \"editor\": \"zed\"\n}").unwrap();
        let err = object(&path).unwrap_err();
        assert!(err.contains("holds a comment"), "{err}");
        std::fs::write(&path, r#"{"editor": "https://x"}"#).unwrap();
        assert!(object(&path).is_ok(), "a URL is no comment");
        std::fs::write(&path, "{").unwrap();
        let err = object(&path).unwrap_err();
        assert!(
            !err.contains("comment") && !err.contains("byte order"),
            "{err}"
        );
        assert!(err.contains(&path.display().to_string()), "{err}");
    }

    #[test]
    fn first_time_is_once_per_warning() {
        assert!(first_time("config_json test warning a"));
        assert!(!first_time("config_json test warning a"));
        assert!(first_time("config_json test warning b"));
    }
}
