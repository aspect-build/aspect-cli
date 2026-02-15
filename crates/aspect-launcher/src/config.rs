use std::collections::HashMap;
use std::env::current_dir;
use std::fmt::Debug;
use std::fs;
use std::path::{Path, PathBuf};

use aspect_telemetry::cargo_pkg_short_version;
use miette::{Result, miette};
use starlark_syntax::syntax::ast::{ArgumentP, AstExpr, AstLiteral, CallArgsP, Expr, Stmt};
use starlark_syntax::syntax::module::AstModuleFields;
use starlark_syntax::syntax::{AstModule, Dialect};

const AXL_MODULE_FILE: &str = "MODULE.aspect";

#[derive(Debug, Clone)]
pub struct AspectLauncherConfig {
    pub aspect_cli: AspectCliConfig,
}

#[derive(Debug, Clone)]
pub struct AspectCliConfig {
    sources: Vec<ToolSource>,
    version: String,
}

#[derive(Debug, Clone)]
pub enum ToolSource {
    GitHub {
        org: String,
        repo: String,
        tag: String,
        artifact: String,
    },

    Http {
        url: String,
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

impl ToolSpec for AspectCliConfig {
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

fn default_cli_sources() -> Vec<ToolSource> {
    vec![ToolSource::GitHub {
        org: "aspect-build".into(),
        repo: "aspect-cli".into(),
        tag: String::new(),
        artifact: String::new(),
    }]
}

fn default_aspect_cli_config() -> AspectCliConfig {
    AspectCliConfig {
        sources: default_cli_sources(),
        version: cargo_pkg_short_version(),
    }
}

pub fn default_config() -> AspectLauncherConfig {
    AspectLauncherConfig {
        aspect_cli: default_aspect_cli_config(),
    }
}

/// Extract a string literal value from an expression.
fn extract_string_literal(expr: &AstExpr) -> Result<&str> {
    match &expr.node {
        Expr::Literal(AstLiteral::String(s)) => Ok(&s.node),
        _ => Err(miette!("expected string literal")),
    }
}

/// Extract named string arguments from a function call's args.
fn extract_named_string_args<'a>(
    args: &'a CallArgsP<starlark_syntax::syntax::ast::AstNoPayload>,
) -> Result<HashMap<&'a str, &'a str>> {
    let mut map = HashMap::new();
    for arg in &args.args {
        match &arg.node {
            ArgumentP::Named(name, expr) => {
                let value = extract_string_literal(expr)?;
                map.insert(name.node.as_str(), value);
            }
            ArgumentP::Positional(_) => {}
            _ => return Err(miette!("unexpected *args or **kwargs in source call")),
        }
    }
    Ok(map)
}

/// Parse a `local(path)` call into ToolSource::Local.
fn parse_local_source(
    args: &CallArgsP<starlark_syntax::syntax::ast::AstNoPayload>,
) -> Result<ToolSource> {
    for arg in &args.args {
        if let ArgumentP::Positional(expr) = &arg.node {
            let path = extract_string_literal(expr)?;
            return Ok(ToolSource::Local {
                path: path.to_owned(),
            });
        }
    }
    Err(miette!("local() requires a path argument"))
}

/// Parse a `github(org, repo, tag?, artifact?)` call into ToolSource::GitHub.
fn parse_github_source(
    args: &CallArgsP<starlark_syntax::syntax::ast::AstNoPayload>,
) -> Result<ToolSource> {
    let named = extract_named_string_args(args)?;
    let org = named
        .get("org")
        .ok_or_else(|| miette!("github() requires 'org' argument"))?;
    let repo = named
        .get("repo")
        .ok_or_else(|| miette!("github() requires 'repo' argument"))?;
    let tag = named.get("tag").unwrap_or(&"");
    let artifact = named.get("artifact").unwrap_or(&"");
    Ok(ToolSource::GitHub {
        org: (*org).to_owned(),
        repo: (*repo).to_owned(),
        tag: (*tag).to_owned(),
        artifact: (*artifact).to_owned(),
    })
}

/// Parse an `http(url, headers?)` call into ToolSource::Http.
fn parse_http_source(
    args: &CallArgsP<starlark_syntax::syntax::ast::AstNoPayload>,
) -> Result<ToolSource> {
    let named = extract_named_string_args(args)?;
    let url = named
        .get("url")
        .ok_or_else(|| miette!("http() requires 'url' argument"))?;

    // Parse headers from the dict expression if present
    let mut headers = HashMap::new();
    for arg in &args.args {
        if let ArgumentP::Named(name, expr) = &arg.node {
            if name.node == "headers" {
                if let Expr::Dict(entries) = &expr.node {
                    for (key_expr, val_expr) in entries {
                        let key = extract_string_literal(key_expr)?;
                        let val = extract_string_literal(val_expr)?;
                        headers.insert(key.to_owned(), val.to_owned());
                    }
                }
            }
        }
    }

    Ok(ToolSource::Http {
        url: (*url).to_owned(),
        headers,
    })
}

/// Parse a source expression (a function call like local(), github(), http()).
fn parse_source(expr: &AstExpr) -> Result<ToolSource> {
    match &expr.node {
        Expr::Call(callee, args) => {
            let name = match &callee.node {
                Expr::Identifier(id) => &id.ident,
                _ => return Err(miette!("expected source function name")),
            };
            match name.as_str() {
                "local" => parse_local_source(args),
                "github" => parse_github_source(args),
                "http" => parse_http_source(args),
                other => Err(miette!(
                    "unknown source type '{}': expected local(), github(), or http()",
                    other
                )),
            }
        }
        _ => Err(miette!(
            "expected a function call (local(), github(), or http()) in sources list"
        )),
    }
}

/// Parse the content of a version.axl file into an AspectLauncherConfig.
fn parse_version_axl(content: &str) -> Result<AspectLauncherConfig> {
    let ast = AstModule::parse("version.axl", content.to_owned(), &Dialect::Standard)
        .map_err(|e| miette!("failed to parse version.axl: {}", e))?;

    let root = ast.statement();

    // Find the version() call - could be directly an Expression or inside Statements
    let version_call = match &root.node {
        Stmt::Expression(expr) => Some(expr),
        Stmt::Statements(stmts) => {
            let mut found = None;
            for stmt in stmts {
                if let Stmt::Expression(expr) = &stmt.node {
                    if let Expr::Call(callee, _) = &expr.node {
                        if let Expr::Identifier(id) = &callee.node {
                            if id.ident == "version" {
                                found = Some(expr);
                                break;
                            }
                        }
                    }
                }
            }
            found
        }
        _ => None,
    };

    let version_expr =
        version_call.ok_or_else(|| miette!("expected a version() call in version.axl"))?;

    let (callee, args) = match &version_expr.node {
        Expr::Call(callee, args) => (callee, args),
        _ => return Err(miette!("expected a version() call in version.axl")),
    };

    // Verify callee is "version"
    match &callee.node {
        Expr::Identifier(id) if id.ident == "version" => {}
        _ => return Err(miette!("expected a version() call in version.axl")),
    }

    let mut version: Option<String> = None;
    let mut sources: Option<Vec<ToolSource>> = None;

    for arg in &args.args {
        match &arg.node {
            ArgumentP::Named(name, expr) => match name.node.as_str() {
                "use" => {
                    version = Some(extract_string_literal(expr)?.to_owned());
                }
                "sources" => {
                    if let Expr::List(items) = &expr.node {
                        let mut src_list = Vec::new();
                        for item in items {
                            src_list.push(parse_source(item)?);
                        }
                        sources = Some(src_list);
                    } else {
                        return Err(miette!("'sources' must be a list"));
                    }
                }
                other => {
                    return Err(miette!("unknown argument '{}' in version() call", other));
                }
            },
            _ => {
                return Err(miette!("version() only accepts named arguments (use, sources)"));
            }
        }
    }

    Ok(AspectLauncherConfig {
        aspect_cli: AspectCliConfig {
            version: version.unwrap_or_else(cargo_pkg_short_version),
            sources: sources.unwrap_or_else(default_cli_sources),
        },
    })
}

pub fn load_config(path: &PathBuf) -> Result<AspectLauncherConfig> {
    let content = match fs::read_to_string(path) {
        Ok(c) => c,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(default_config()),
        Err(e) => return Err(miette!("failed to read config file {:?}: {}", path, e)),
    };

    parse_version_axl(&content)
}

/// Automatically determines the project root directory and loads the Aspect configuration.
///
/// The root dir is identified as the first (deepest) ancestor directory of the current working
/// directory that contains at least one of the following boundary files: MODULE.aspect, MODULE.bazel,
/// MODULE.bazel.lock, REPO.bazel, WORKSPACE, or WORKSPACE.bazel. If no such directory is found, the
/// current working directory is used as the project root.
///
/// It then constructs the path to `.aspect/version.axl` within the project root directory and loads the
/// configuration using `load_config`.
///
/// **Returns**
///
/// A `Result` containing a tuple `(PathBuf, AspectLauncherConfig)` where:
/// - The first element is the determined root directory.
/// - The second element is the loaded `AspectLauncherConfig`.
///
/// **Errors**
///
/// Returns an error if the current working directory cannot be obtained or if loading the config fails.
pub fn autoconf() -> Result<(PathBuf, AspectLauncherConfig)> {
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

    let version_axl = root_dir
        .join(PathBuf::from(".aspect/version.axl"))
        .to_path_buf();
    let config = load_config(&version_axl)?;
    Ok((root_dir, config))
}
