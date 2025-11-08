use std::env::current_dir;
use std::path::{Path, PathBuf};
use std::{collections::HashMap, fmt::Debug, fs};

use aspect_telemetry::cargo_pkg_short_version;
use miette::{miette, Result};
use serde::Deserialize;

const AXL_MODULE_FILE: &str = "MODULE.aspect";

#[derive(Deserialize, Debug, Clone)]
pub struct AspectConfig {
    pub tools: ToolsConfig,
}

#[derive(Deserialize, Debug, Clone)]
pub struct ToolsConfig {
    pub cli: CliConfig,
    pub bazelisk: BazeliskConfig,
}

fn default_cli_sources() -> Vec<ToolSource> {
    vec![{
        ToolSource::Github {
            org: "aspect-build".into(),
            repo: "aspect-cli".into(),
            release: "v{{ version }}".into(),
            artifact: "aspect-cli-{{ llvm_triple }}".into(),
        }
    }]
}

#[derive(Deserialize, Debug, Clone)]
pub struct CliConfig {
    #[serde(default = "default_cli_sources")]
    sources: Vec<ToolSource>,
    #[serde(default = "cargo_pkg_short_version")]
    version: String,
}

fn default_bazelisk_sources() -> Vec<ToolSource> {
    vec![{
        ToolSource::Http {
            url: "https://github.com/bazelbuild/bazelisk/releases/download/v{{ version }}/bazelisk-{{ goos }}-{{ goarch }}".into(),
            headers: HashMap::new(),
        }
    }]
}

fn default_bazelisk_version() -> String {
    "1.27.0".into()
}

#[derive(Deserialize, Debug, Clone)]
pub struct BazeliskConfig {
    #[serde(default = "default_bazelisk_sources")]
    sources: Vec<ToolSource>,
    #[serde(default = "default_bazelisk_version")]
    version: String,
}

#[derive(Deserialize, Debug, Clone)]
#[serde(untagged)]
pub enum ToolSource {
    Github {
        org: String,
        repo: String,
        release: String,
        artifact: String,
    },

    Http {
        url: String,
        #[serde(default = "HashMap::new")]
        headers: HashMap<String, String>,
    },

    Local {
        path: String,
    },
}

pub trait ToolSpec: Debug {
    fn name(&self) -> String;
    fn version(&self) -> &String;
    fn sources(&self) -> &Vec<ToolSource>;
}

impl ToolSpec for CliConfig {
    fn name(&self) -> String {
        "aspect-cli".to_owned()
    }

    fn sources(&self) -> &Vec<ToolSource> {
        &self.sources
    }

    fn version(&self) -> &String {
        &self.version
    }
}

impl ToolSpec for BazeliskConfig {
    fn name(&self) -> String {
        "bazelisk".to_owned()
    }

    fn sources(&self) -> &Vec<ToolSource> {
        &self.sources
    }

    fn version(&self) -> &String {
        &self.version
    }
}

pub fn load_config(path: &PathBuf) -> Result<AspectConfig> {
    let content = match fs::read_to_string(path) {
        Ok(c) => c,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(default_config()),
        Err(e) => return Err(miette!("failed to read config file {:?}: {}", path, e)),
    };

    match toml::from_str(&content) {
        Ok(config) => Ok(config),
        Err(e) => Err(miette!("failed to parse config file {:?}: {}", path, e)),
    }
}

pub fn default_config() -> AspectConfig {
    // FIXME: Better way to fall back to an empty config?
    toml::from_str("[tools.cli]\n[tools.bazelisk]\n").unwrap()
}

/// Automatically determines the project root directory and loads the Aspect configuration.
///
/// This function starts from the current working directory and searches upwards through its ancestors
/// for repository boundary marker files (such as `AXL_MODULE_FILE`, `MODULE.bazel`, `MODULE.bazel.lock`,
/// `REPO.bazel`, `WORKSPACE`, or `WORKSPACE.bazel`). The first ancestor directory containing any of
/// these files is considered the project root. If no such directory is found, the current directory
/// is used as the root.
///
/// It then constructs the path to `.aspect/config.toml` within the root directory and loads the
/// configuration using `load_config`.
///
/// # Returns
///
/// A `Result` containing a tuple `(PathBuf, AspectConfig)` where:
/// - The first element is the determined root directory.
/// - The second element is the loaded `AspectConfig`.
///
/// # Errors
///
/// Returns an error if the current working directory cannot be obtained or if loading the config fails.
pub fn autoconf() -> Result<(PathBuf, AspectConfig)> {
    let current_dir =
        current_dir().map_err(|e| miette!("failed to get current directory: {}", e))?;

    let root_dir = if let Some(repo_root) = current_dir
        .ancestors()
        .filter(|dir| {
            dir.join(PathBuf::from(AXL_MODULE_FILE)).exists()
                // Repository boundary marker files: https://bazel.build/external/overview#repository
                || dir.join(PathBuf::from("MODULE.bazel")).exists()
                || dir.join(PathBuf::from("MODULE.bazel.lock")).exists()
                || dir.join(PathBuf::from("REPO.bazel")).exists()
                || dir.join(PathBuf::from("WORKSPACE")).exists()
                || dir.join(PathBuf::from("WORKSPACE.bazel")).exists()
        })
        .next()
        .map(Path::to_path_buf)
    {
        repo_root
    } else {
        current_dir
    };

    let config_toml = root_dir
        .join(PathBuf::from(".aspect/config.toml"))
        .to_path_buf();
    let config = load_config(&config_toml)?;
    Ok((root_dir, config))
}
