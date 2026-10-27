//! `bazel run`, done by building the target and then running what it built:
//! what the `runinfo` aspect records about a target, and which of its outputs
//! is the executable.

use std::path::{Path, PathBuf};

use serde::Deserialize;

/// Extensions an executable output may carry when the rule names its
/// entrypoint after the target plus a suffix, most to least common.
const SCRIPT_EXTENSIONS: &[&str] = &[".sh", ".bash", ".exe", ".bat"];

/// What the `runinfo` aspect writes per runnable target.
#[derive(Debug, Deserialize, Default)]
pub struct RunInfo {
    #[serde(default)]
    pub executable: String,
    #[serde(default)]
    pub args: Vec<String>,
    #[serde(default)]
    pub environment: serde_json::Map<String, serde_json::Value>,
    #[serde(default)]
    pub inherited_environment: Vec<String>,
    #[serde(default)]
    pub tags: Vec<String>,
    /// The target has runfiles support (a non-empty default runfiles set).
    #[serde(default = "yes")]
    pub runfiles: bool,
    /// Bazel wrote `<exe>.runfiles_manifest` for it.
    #[serde(default = "yes")]
    pub runfiles_manifest: bool,
}

fn yes() -> bool {
    true
}

/// The executable among `candidates`: the aspect's `files_to_run.executable`
/// basename when the aspect ran, else a file named after the target, else
/// the target name plus a script extension, else the first file with the
/// executable bit.
pub fn pick_executable(
    candidates: &[PathBuf],
    runinfo: Option<&RunInfo>,
    target_name: &str,
) -> Option<PathBuf> {
    let basename = |p: &PathBuf| p.file_name().map(|n| n.to_string_lossy().into_owned());
    if let Some(info) = runinfo {
        let exe = info.executable.rsplit('/').next().unwrap_or("");
        if !exe.is_empty() {
            if let Some(found) = candidates
                .iter()
                .find(|c| basename(c).as_deref() == Some(exe))
            {
                return Some(found.clone());
            }
        }
    }
    if let Some(found) = candidates
        .iter()
        .find(|c| basename(c).as_deref() == Some(target_name))
    {
        return Some(found.clone());
    }
    for ext in SCRIPT_EXTENSIONS {
        let name = format!("{target_name}{ext}");
        if let Some(found) = candidates
            .iter()
            .find(|c| basename(c).as_deref() == Some(&name))
        {
            return Some(found.clone());
        }
    }
    candidates.iter().find(|c| is_executable(c)).cloned()
}

#[cfg(unix)]
fn is_executable(path: &Path) -> bool {
    use std::os::unix::fs::PermissionsExt;
    std::fs::metadata(path)
        .map(|m| m.is_file() && m.permissions().mode() & 0o111 != 0)
        .unwrap_or(false)
}

#[cfg(not(unix))]
fn is_executable(path: &Path) -> bool {
    path.is_file()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn executable_choice_prefers_the_aspect_then_the_name_then_extensions() {
        let tmp = tempfile::tempdir().unwrap();
        let mk = |name: &str, exec: bool| {
            let p = tmp.path().join(name);
            std::fs::write(&p, "").unwrap();
            #[cfg(unix)]
            {
                use std::os::unix::fs::PermissionsExt;
                std::fs::set_permissions(
                    &p,
                    std::fs::Permissions::from_mode(if exec { 0o755 } else { 0o644 }),
                )
                .unwrap();
            }
            let _ = exec;
            p
        };
        let other = mk("other", true);
        let tool_sh = mk("tool.sh", false);
        let tool = mk("tool", false);
        let candidates = vec![other.clone(), tool_sh.clone(), tool.clone()];
        let info = RunInfo {
            executable: "pkg/tool.sh".to_owned(),
            ..Default::default()
        };
        assert_eq!(
            pick_executable(&candidates, Some(&info), "tool"),
            Some(tool_sh.clone())
        );
        assert_eq!(pick_executable(&candidates, None, "tool"), Some(tool));
        assert_eq!(
            pick_executable(&[other.clone(), tool_sh.clone()], None, "tool"),
            Some(tool_sh)
        );
        #[cfg(unix)]
        assert_eq!(pick_executable(&[other.clone()], None, "tool"), Some(other));
    }

    #[test]
    fn runinfo_json_decodes_what_the_aspect_writes() {
        let text = r#"{"executable":"pkg/tool","args":["a","b c"],"environment":{"K":"V"},"inherited_environment":["HOME"],"tags":["manual"]}"#;
        let info: RunInfo = serde_json::from_str(text).unwrap();
        assert_eq!(info.executable, "pkg/tool");
        assert_eq!(info.args, vec!["a", "b c"]);
        assert_eq!(info.environment.get("K").unwrap(), "V");
        assert_eq!(info.inherited_environment, vec!["HOME"]);
        assert_eq!(info.tags, vec!["manual"]);
        // Fields an older aspect did not write default to the common case.
        assert!(info.runfiles && info.runfiles_manifest);
        let text = r#"{"executable":"pkg/tool","runfiles":true,"runfiles_manifest":false}"#;
        let info: RunInfo = serde_json::from_str(text).unwrap();
        assert!(info.runfiles && !info.runfiles_manifest);
    }
}
