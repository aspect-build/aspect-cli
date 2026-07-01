use std::collections::HashMap;
use std::env::current_dir;
use std::fmt::Debug;
use std::fs;
use std::path::{Path, PathBuf};

use miette::{Result, miette};
use starlark_syntax::syntax::ast::{ArgumentP, AstExpr, AstLiteral, CallArgsP, Expr, Stmt};
use starlark_syntax::syntax::{AstModule, Dialect};

const AXL_MODULE_FILE: &str = "MODULE.aspect";
const AXL_VERSION_AXL_REL: &str = ".aspect/version.axl";

/// Boundary files identifying an Aspect project root. Probed before the Bazel
/// markers so a nested Aspect workspace inside a Bazel monorepo (e.g.
/// `/mono/proj/.aspect/version.axl` under `/mono/MODULE.bazel`) resolves to
/// the Aspect workspace, not the outer Bazel one.
const ASPECT_BOUNDARY_FILES: &[&str] = &[AXL_MODULE_FILE, AXL_VERSION_AXL_REL];

/// Bazel repository boundary marker files (see
/// https://bazel.build/external/overview#repository), probed only when no
/// Aspect boundary file is found.
const BAZEL_BOUNDARY_FILES: &[&str] = &[
    "MODULE.bazel",
    "MODULE.bazel.lock",
    "REPO.bazel",
    "WORKSPACE",
    "WORKSPACE.bazel",
];

/// Walk ancestors of `start` and return the deepest one containing any of
/// `markers`, skipping any ancestor equal to `except`.
///
/// `except` guards against the user-level `~/.aspect/version.axl` being mistaken
/// for a project boundary: it shares the [`AXL_VERSION_AXL_REL`] path with a
/// project marker but must only supply config, never define the root.
fn find_ancestor_with_any(
    start: &Path,
    markers: &[&str],
    except: Option<&Path>,
) -> Option<PathBuf> {
    start
        .ancestors()
        .filter(|dir| except != Some(*dir))
        .find(|dir| markers.iter().any(|m| dir.join(m).exists()))
        .map(Path::to_path_buf)
}

#[derive(Debug, Clone)]
pub struct AspectLauncherConfig {
    pub aspect_cli: AspectCliConfig,
}

#[derive(Debug, Clone)]
pub struct AspectCliConfig {
    sources: Vec<ToolSource>,
    /// The pinned version string, or `None` if the version should be resolved
    /// by querying the releases API for the latest available release.
    version: Option<String>,
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
    /// The pinned version string, or `None` when the version should be resolved
    /// at download time (e.g. by querying the GitHub releases API).
    fn version(&self) -> Option<&str>;
    fn sources(&self) -> &Vec<ToolSource>;
}

impl ToolSpec for AspectCliConfig {
    fn name(&self) -> String {
        "aspect-cli".to_owned()
    }

    fn sources(&self) -> &Vec<ToolSource> {
        &self.sources
    }

    fn version(&self) -> Option<&str> {
        self.version.as_deref()
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
        version: None,
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
            ArgumentP::Positional(expr) => {
                if version.is_none() {
                    version = Some(extract_string_literal(expr)?.to_owned());
                } else {
                    return Err(miette!(
                        "version() accepts only one positional argument (the version string)"
                    ));
                }
            }
            ArgumentP::Named(name, expr) => match name.node.as_str() {
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
                return Err(miette!("unexpected *args or **kwargs in version() call"));
            }
        }
    }

    Ok(AspectLauncherConfig {
        aspect_cli: AspectCliConfig {
            version,
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_version_with_pinned_version_and_github_source() {
        let content = r#"
version(
    "2026.11.6",
    sources = [
        github(
            org = "aspect-build",
            repo = "aspect-cli",
        ),
    ],
)
"#;
        let config = parse_version_axl(content).unwrap();
        assert_eq!(config.aspect_cli.version(), Some("2026.11.6"));
        assert_eq!(config.aspect_cli.sources().len(), 1);
        match &config.aspect_cli.sources()[0] {
            ToolSource::GitHub {
                org,
                repo,
                tag,
                artifact,
            } => {
                assert_eq!(org, "aspect-build");
                assert_eq!(repo, "aspect-cli");
                assert_eq!(tag, "");
                assert_eq!(artifact, "");
            }
            other => panic!("expected GitHub source, got {:?}", other),
        }
    }

    #[test]
    fn test_parse_version_with_custom_tag_and_artifact() {
        let content = r#"
version(
    "1.2.3",
    sources = [
        github(
            org = "my-org",
            repo = "my-repo",
            tag = "release-{version}",
            artifact = "my-tool-{target}",
        ),
    ],
)
"#;
        let config = parse_version_axl(content).unwrap();
        assert_eq!(config.aspect_cli.version(), Some("1.2.3"));
        match &config.aspect_cli.sources()[0] {
            ToolSource::GitHub { tag, artifact, .. } => {
                assert_eq!(tag, "release-{version}");
                assert_eq!(artifact, "my-tool-{target}");
            }
            other => panic!("expected GitHub source, got {:?}", other),
        }
    }

    #[test]
    fn test_parse_version_with_no_version_uses_default() {
        let content = r#"version()"#;
        let config = parse_version_axl(content).unwrap();
        assert_eq!(config.aspect_cli.version(), None);
    }

    #[test]
    fn test_parse_version_with_custom_sources_but_no_version() {
        let content = r#"
version(
    sources = [
        local("bazel-bin/cli/aspect"),
        github(org = "my-fork", repo = "aspect-cli"),
    ],
)
"#;
        let config = parse_version_axl(content).unwrap();
        assert_eq!(config.aspect_cli.version(), None);
        assert_eq!(config.aspect_cli.sources().len(), 2);
        assert!(matches!(
            &config.aspect_cli.sources()[0],
            ToolSource::Local { path } if path == "bazel-bin/cli/aspect"
        ));
        match &config.aspect_cli.sources()[1] {
            ToolSource::GitHub { org, repo, .. } => {
                assert_eq!(org, "my-fork");
                assert_eq!(repo, "aspect-cli");
            }
            other => panic!("expected GitHub source, got {:?}", other),
        }
    }

    #[test]
    fn test_parse_version_with_http_source() {
        let content = r#"
version(
    "1.0.0",
    sources = [
        http(
            url = "https://example.com/tool-{version}-{target}",
        ),
    ],
)
"#;
        let config = parse_version_axl(content).unwrap();
        match &config.aspect_cli.sources()[0] {
            ToolSource::Http { url, headers } => {
                assert_eq!(url, "https://example.com/tool-{version}-{target}");
                assert!(headers.is_empty());
            }
            other => panic!("expected Http source, got {:?}", other),
        }
    }

    #[test]
    fn test_parse_version_with_http_source_headers_is_broken() {
        // NOTE: extract_named_string_args fails on non-string named args
        // like `headers = {...}`. This is a known bug.
        let content = r#"
version(
    "1.0.0",
    sources = [
        http(
            url = "https://example.com/tool",
            headers = {"Authorization": "Bearer token"},
        ),
    ],
)
"#;
        let result = parse_version_axl(content);
        assert!(result.is_err(), "http() with headers is currently broken");
    }

    #[test]
    fn test_parse_version_with_local_source() {
        let content = r#"
version(
    "1.0.0",
    sources = [
        local("bazel-bin/cli/aspect"),
    ],
)
"#;
        let config = parse_version_axl(content).unwrap();
        match &config.aspect_cli.sources()[0] {
            ToolSource::Local { path } => {
                assert_eq!(path, "bazel-bin/cli/aspect");
            }
            other => panic!("expected Local source, got {:?}", other),
        }
    }

    #[test]
    fn test_parse_version_with_multiple_sources() {
        let content = r#"
version(
    "1.0.0",
    sources = [
        local("bazel-bin/cli/aspect"),
        github(org = "aspect-build", repo = "aspect-cli"),
    ],
)
"#;
        let config = parse_version_axl(content).unwrap();
        assert_eq!(config.aspect_cli.sources().len(), 2);
        assert!(matches!(
            &config.aspect_cli.sources()[0],
            ToolSource::Local { .. }
        ));
        assert!(matches!(
            &config.aspect_cli.sources()[1],
            ToolSource::GitHub { .. }
        ));
    }

    #[test]
    fn test_parse_version_no_sources_uses_default() {
        let content = r#"version("1.0.0")"#;
        let config = parse_version_axl(content).unwrap();
        assert_eq!(config.aspect_cli.sources().len(), 1);
        match &config.aspect_cli.sources()[0] {
            ToolSource::GitHub { org, repo, .. } => {
                assert_eq!(org, "aspect-build");
                assert_eq!(repo, "aspect-cli");
            }
            other => panic!("expected default GitHub source, got {:?}", other),
        }
    }

    #[test]
    fn test_parse_version_missing_version_call_errors() {
        let content = r#"print("hello")"#;
        let result = parse_version_axl(content);
        assert!(result.is_err());
    }

    #[test]
    fn test_parse_version_invalid_syntax_errors() {
        let content = r#"version(123)"#;
        let result = parse_version_axl(content);
        assert!(result.is_err());
    }

    #[test]
    fn test_parse_version_unknown_argument_errors() {
        let content = r#"version("1.0.0", flavor = "spicy")"#;
        let result = parse_version_axl(content);
        assert!(result.is_err());
    }

    #[test]
    fn test_parse_version_duplicate_positional_errors() {
        let content = r#"version("1.0.0", "2.0.0")"#;
        let result = parse_version_axl(content);
        assert!(result.is_err());
    }

    #[test]
    fn test_parse_github_missing_org_errors() {
        let content = r#"version("1.0.0", sources = [github(repo = "aspect-cli")])"#;
        let result = parse_version_axl(content);
        assert!(result.is_err());
    }

    #[test]
    fn test_parse_github_missing_repo_errors() {
        let content = r#"version("1.0.0", sources = [github(org = "aspect-build")])"#;
        let result = parse_version_axl(content);
        assert!(result.is_err());
    }

    #[test]
    fn test_parse_unknown_source_type_errors() {
        let content = r#"version("1.0.0", sources = [ftp("foo")])"#;
        let result = parse_version_axl(content);
        assert!(result.is_err());
    }

    #[test]
    fn test_default_config() {
        let config = super::default_config();
        assert_eq!(config.aspect_cli.version(), None);
        assert_eq!(config.aspect_cli.sources().len(), 1);
        assert!(matches!(
            &config.aspect_cli.sources()[0],
            ToolSource::GitHub {
                org,
                repo,
                ..
            } if org == "aspect-build" && repo == "aspect-cli"
        ));
    }

    /// Resolve the project root from `start`, replicating `autoconf`'s
    /// two-pass walk without touching the process cwd or filesystem read of
    /// `.aspect/version.axl`. `home` is excluded from the Aspect pass, mirroring
    /// how `autoconf` keeps `~/.aspect/version.axl` out of root discovery.
    fn resolve_root(start: &Path, home: Option<&Path>) -> PathBuf {
        find_ancestor_with_any(start, ASPECT_BOUNDARY_FILES, home)
            .or_else(|| find_ancestor_with_any(start, BAZEL_BOUNDARY_FILES, None))
            .unwrap_or_else(|| start.to_path_buf())
    }

    /// Aspect markers anywhere in the ancestor chain take precedence over a
    /// Bazel marker higher up — that's the sub-workspace case.
    #[test]
    fn root_prefers_aspect_marker_over_outer_bazel_marker() {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path();
        std::fs::write(root.join("MODULE.bazel"), "").unwrap();
        let nested = root.join("proj");
        std::fs::create_dir_all(nested.join(".aspect")).unwrap();
        std::fs::write(nested.join(".aspect/version.axl"), "").unwrap();
        let cwd = nested.join("src");
        std::fs::create_dir_all(&cwd).unwrap();

        assert_eq!(resolve_root(&cwd, None), nested);
    }

    /// MODULE.aspect is a valid Aspect marker too.
    #[test]
    fn root_resolves_to_module_aspect() {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path();
        std::fs::write(root.join("MODULE.aspect"), "").unwrap();
        let cwd = root.join("sub/dir");
        std::fs::create_dir_all(&cwd).unwrap();

        assert_eq!(resolve_root(&cwd, None), root);
    }

    /// Pure Bazel monorepo with no Aspect markers: fall back to Bazel markers.
    #[test]
    fn root_falls_back_to_bazel_markers() {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path();
        std::fs::write(root.join("MODULE.bazel"), "").unwrap();
        let cwd = root.join("sub/dir");
        std::fs::create_dir_all(&cwd).unwrap();

        assert_eq!(resolve_root(&cwd, None), root);
    }

    /// The home directory's `~/.aspect/version.axl` must not act as a project
    /// root: a Bazel repo under `$HOME` still resolves to the repo, not `$HOME`.
    #[test]
    fn root_ignores_home_version_axl_for_a_repo_under_home() {
        let home = tempfile::tempdir().unwrap();
        write_version_axl(home.path(), "9.9.9");
        let repo = home.path().join("work/repo");
        std::fs::create_dir_all(&repo).unwrap();
        std::fs::write(repo.join("MODULE.bazel"), "").unwrap();
        let cwd = repo.join("sub/dir");
        std::fs::create_dir_all(&cwd).unwrap();

        assert_eq!(resolve_root(&cwd, Some(home.path())), repo);
    }

    /// No markers anywhere → fall back to the starting directory itself.
    #[test]
    fn root_falls_back_to_start_when_no_markers() {
        let tmp = tempfile::tempdir().unwrap();
        let cwd = tmp.path().join("sub/dir");
        std::fs::create_dir_all(&cwd).unwrap();

        assert_eq!(resolve_root(&cwd, None), cwd);
    }

    fn write_version_axl(dir: &Path, version: &str) {
        let path = dir.join(AXL_VERSION_AXL_REL);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, format!("version(\"{version}\")\n")).unwrap();
    }

    /// The root's `.aspect/version.axl` wins over the home fallback.
    #[test]
    fn config_prefers_root_over_home() {
        let root = tempfile::tempdir().unwrap();
        let home = tempfile::tempdir().unwrap();
        write_version_axl(root.path(), "1.0.0");
        write_version_axl(home.path(), "9.9.9");

        let config = resolve_config(root.path(), Some(home.path())).unwrap();
        assert_eq!(config.aspect_cli.version(), Some("1.0.0"));
    }

    /// Missing root config falls back to `~/.aspect/version.axl`.
    #[test]
    fn config_falls_back_to_home() {
        let root = tempfile::tempdir().unwrap();
        let home = tempfile::tempdir().unwrap();
        write_version_axl(home.path(), "9.9.9");

        let config = resolve_config(root.path(), Some(home.path())).unwrap();
        assert_eq!(config.aspect_cli.version(), Some("9.9.9"));
    }

    /// No root config and no home config → default config.
    #[test]
    fn config_defaults_when_nothing_found() {
        let root = tempfile::tempdir().unwrap();
        let home = tempfile::tempdir().unwrap();

        let config = resolve_config(root.path(), Some(home.path())).unwrap();
        assert_eq!(
            config.aspect_cli.version(),
            default_config().aspect_cli.version()
        );
    }

    /// A missing home directory is tolerated and yields the default config.
    #[test]
    fn config_defaults_when_home_unknown() {
        let root = tempfile::tempdir().unwrap();

        let config = resolve_config(root.path(), None).unwrap();
        assert_eq!(
            config.aspect_cli.version(),
            default_config().aspect_cli.version()
        );
    }
}

/// Automatically determines the project root directory and loads the Aspect configuration.
///
/// Two-pass walk over the ancestors of the current working directory, deepest
/// to shallowest:
/// 1. Returns the first ancestor containing any [`ASPECT_BOUNDARY_FILES`] entry
///    (`MODULE.aspect` or `.aspect/version.axl`), excluding the home directory
///    so the user-level `~/.aspect/version.axl` never acts as a project root.
/// 2. If none found, returns the first ancestor containing any [`BAZEL_BOUNDARY_FILES`] entry.
/// 3. If still none found, the current working directory is used.
///
/// The Aspect-first ordering lets a nested Aspect workspace inside a Bazel
/// monorepo opt out of the surrounding Bazel root by dropping a `.aspect/`
/// directory or a `MODULE.aspect` file at its boundary.
///
/// The config for the resolved root is loaded via [`resolve_config`], which
/// falls back to `~/.aspect/version.axl` and then the built-in default. The
/// returned root directory is unaffected by that fallback.
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

    let home_dir = dirs::home_dir();

    let root_dir = find_ancestor_with_any(&current_dir, ASPECT_BOUNDARY_FILES, home_dir.as_deref())
        .or_else(|| find_ancestor_with_any(&current_dir, BAZEL_BOUNDARY_FILES, None))
        .unwrap_or(current_dir);

    let config = resolve_config(&root_dir, home_dir.as_deref())?;
    Ok((root_dir, config))
}

/// Load the launcher config for a resolved `root_dir`, falling back to the
/// user-level home directory before defaulting.
///
/// Resolution order:
/// 1. `.aspect/version.axl` under `root_dir`, if it exists.
/// 2. `.aspect/version.axl` under `home_dir`, if a home dir is known and it exists.
/// 3. [`default_config`].
fn resolve_config(root_dir: &Path, home_dir: Option<&Path>) -> Result<AspectLauncherConfig> {
    let version_axl = root_dir.join(AXL_VERSION_AXL_REL);
    if version_axl.exists() {
        return load_config(&version_axl);
    }

    if let Some(home_version_axl) = home_dir.map(|h| h.join(AXL_VERSION_AXL_REL))
        && home_version_axl.exists()
    {
        return load_config(&home_version_axl);
    }

    Ok(default_config())
}
