use std::env::current_dir;
use std::path::{Path, PathBuf};
use std::{collections::HashMap, fmt::Debug, fs};

use aspect_telemetry::cargo_pkg_version;
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
            artifact: "aspect-cli_{{ llvm_triple }}".into(),
        }
    }]
}

#[derive(Deserialize, Debug, Clone)]
pub struct CliConfig {
    #[serde(default = "default_cli_sources")]
    sources: Vec<ToolSource>,
    #[serde(default = "cargo_pkg_version")]
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

pub fn load_config(path: &PathBuf) -> AspectConfig {
    if fs::exists(path).is_ok()
        && let Ok(content) = fs::read_to_string(path)
    {
        // FIXME: How to handle decode errors here?
        if let Ok(config) = toml::from_str(content.as_str()) {
            return config;
        }
    }
    // FIXME: Better way to fall back to an empty config?
    toml::from_str("[tools.cli]\n[tools.bazelisk]\n").unwrap()
}

pub fn autoconf() -> (PathBuf, AspectConfig) {
    let Some(repo_dir) = current_dir()
        .unwrap()
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
    else {
        panic!("unable to identify a repository root dir");
    };

    let aspect_telemetry = repo_dir
        .join(PathBuf::from(".aspect/config.toml"))
        .to_path_buf();
    (repo_dir, load_config(&aspect_telemetry))
}
