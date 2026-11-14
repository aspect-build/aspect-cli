use std::path::{Path, PathBuf};

use axl_runtime::eval::{self, LoadPath};
use axl_runtime::module::DiskStore;
use starlark::analysis::AstModuleLint;
use starlark::analysis::EvalSeverity;
use starlark::docs::DocModule;
use starlark::errors::EvalMessage;
use starlark::syntax::AstModule;
use starlark_lsp::error::eval_message_to_lsp_diagnostic;
use starlark_lsp::server::LspContext;
use starlark_lsp::server::LspEvalResult;
use starlark_lsp::server::LspUri;
use starlark_lsp::server::StringLiteralResult;

pub struct AxlContext {}

impl AxlContext {
    fn resolve_load_to_path(
        &self,
        path: &str,
        current_file: &Path,
        workspace_root: Option<&Path>,
    ) -> Result<PathBuf, String> {
        let current_dir = current_file
            .parent()
            .ok_or_else(|| format!("{} has no parent directory", current_file.display()))?;

        if crate::bazel::is_bazel_label(path) {
            return crate::bazel::resolve_label(path, current_dir)
                .or_else(|_| {
                    crate::bazel::best_candidate(path, current_dir)
                        .ok_or_else(|| "no candidates for bazel label".to_owned())
                })
                .map(|p| normalize(&p));
        }

        let load_path = LoadPath::try_from(path).map_err(|e| e.to_string())?;

        let candidates: Vec<PathBuf> = match &load_path {
            LoadPath::RelativePath(rel) => vec![current_dir.join(rel)],

            LoadPath::ModuleSpecifier { module, subpath } => {
                let mut cands = Vec::new();
                if let Some(root) = workspace_root {
                    cands.push(root.join(".aspect/modules").join(module).join(subpath));
                    let store = DiskStore::new(root.to_path_buf());
                    cands.push(store.deps_path().join(module).join(subpath));
                    cands.push(store.builtins_path().join(module).join(subpath));
                }
                cands
            }

            LoadPath::ModuleSubpath(sub) => {
                let mut cands = Vec::new();
                let mut dir = Some(current_dir);
                while let Some(d) = dir {
                    cands.push(d.join(sub));
                    if workspace_root.is_some_and(|root| d == root) {
                        break;
                    }
                    dir = d.parent();
                }
                cands
            }
        };

        candidates
            .iter()
            .find(|c| c.is_file())
            .map(|c| normalize(c))
            .ok_or_else(|| {
                format!(
                    "cannot resolve load(\"{path}\"): tried {}",
                    candidates
                        .iter()
                        .map(|c| c.display().to_string())
                        .collect::<Vec<_>>()
                        .join(", ")
                )
            })
    }
}

fn clarify_scope_message(path: &Path, problem: &str) -> Option<String> {
    let name = problem.split('`').nth(1)?;
    if crate::bazel::is_bazel_file(path) {
        let home = crate::bazel::available_in(name)?;
        let here = crate::bazel::kind_description(path);
        let hint = if here == ".bzl files" && crate::bazel::is_build_native(name) {
            format!(" (from .bzl, call it as `native.{name}`)")
        } else {
            String::new()
        };
        return Some(format!(
            "`{name}` is not available in {here}: it can only be used in {home}{hint}"
        ));
    }
    let file_name = path.file_name()?.to_str()?;
    let in_manifest = manifest_global_names().contains(name);
    let in_engine = engine_global_names().contains(name);
    match file_name {
        "MODULE.aspect" => in_engine.then(|| {
            format!("`{name}` is not available in MODULE.aspect: it can only be used in .axl module files")
        }),
        "version.axl" => (in_engine || in_manifest).then(|| {
            format!("`{name}` is not available in version.axl: only `version(\"...\")` is allowed here")
        }),
        _ => in_manifest.then(|| {
            format!("`{name}` is not available in .axl files: it can only be used in MODULE.aspect")
        }),
    }
}

fn manifest_global_names() -> &'static std::collections::HashSet<String> {
    static NAMES: std::sync::OnceLock<std::collections::HashSet<String>> =
        std::sync::OnceLock::new();
    NAMES.get_or_init(|| {
        axl_runtime::module::manifest_globals()
            .documentation()
            .members
            .keys()
            .cloned()
            .collect()
    })
}

fn engine_global_names() -> &'static std::collections::HashSet<String> {
    static NAMES: std::sync::OnceLock<std::collections::HashSet<String>> =
        std::sync::OnceLock::new();
    NAMES.get_or_init(|| {
        eval::get_globals()
            .build()
            .documentation()
            .members
            .keys()
            .cloned()
            .collect()
    })
}

fn normalize(path: &Path) -> PathBuf {
    let mut out = PathBuf::new();
    for component in path.components() {
        match component {
            std::path::Component::CurDir => {}
            std::path::Component::ParentDir => {
                out.pop();
            }
            other => out.push(other),
        }
    }
    out
}

impl LspContext for AxlContext {
    fn parse_file_with_contents(&self, uri: &LspUri, content: String) -> LspEvalResult {
        match uri {
            LspUri::File(path) => {
                match AstModule::parse(&path.to_string_lossy(), content, &eval::api::dialect()) {
                    Ok(ast) => {
                        let globals: std::collections::HashSet<String> =
                            self.get_environment(uri).members.keys().cloned().collect();
                        let diagnostics = ast
                            .lint(Some(&globals))
                            .into_iter()
                            .filter(|lint| {
                                matches!(
                                    lint.short_name.as_str(),
                                    "using-undefined"
                                        | "using-unassigned"
                                        | "using-maybe-undefined"
                                )
                            })
                            .map(|mut lint| {
                                lint.severity = match lint.short_name.as_str() {
                                    "using-maybe-undefined" => EvalSeverity::Warning,
                                    _ => EvalSeverity::Error,
                                };
                                if let Some(better) = clarify_scope_message(path, &lint.problem) {
                                    lint.problem = better;
                                }
                                eval_message_to_lsp_diagnostic(EvalMessage::from(lint))
                            })
                            .collect();
                        LspEvalResult {
                            diagnostics,
                            ast: Some(ast),
                        }
                    }
                    Err(err) => {
                        let err = EvalMessage::from_error(path, &err);
                        LspEvalResult {
                            diagnostics: vec![eval_message_to_lsp_diagnostic(err)],
                            ast: None,
                        }
                    }
                }
            }
            _ => LspEvalResult::default(),
        }
    }

    fn resolve_load(
        &self,
        path: &str,
        current_file: &LspUri,
        workspace_root: Option<&Path>,
    ) -> Result<LspUri, String> {
        match current_file {
            LspUri::File(current) => self
                .resolve_load_to_path(path, current, workspace_root)
                .map(LspUri::File),
            other => Err(format!("unsupported URL scheme: {other}")),
        }
    }

    fn render_as_load(
        &self,
        target: &LspUri,
        current_file: &LspUri,
        _workspace_root: Option<&Path>,
    ) -> Result<String, String> {
        let (LspUri::File(target), LspUri::File(current)) = (target, current_file) else {
            return Err("unsupported URL scheme".to_owned());
        };
        let current_dir = current
            .parent()
            .ok_or_else(|| format!("{} has no parent directory", current.display()))?;
        match target.strip_prefix(current_dir) {
            Ok(rel) => Ok(format!("./{}", rel.display())),
            Err(_) => Ok(target.display().to_string()),
        }
    }

    fn resolve_string_literal(
        &self,
        _literal: &str,
        _current_file: &LspUri,
        _workspace_root: Option<&Path>,
    ) -> Result<Option<StringLiteralResult>, String> {
        Ok(None)
    }

    fn get_load_contents(&self, uri: &LspUri) -> Result<Option<String>, String> {
        match uri {
            LspUri::File(path) => match std::fs::read_to_string(path) {
                Ok(contents) => Ok(Some(contents)),
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
                Err(e) => Err(format!("cannot read {}: {e}", path.display())),
            },
            _ => Ok(None),
        }
    }

    fn get_environment(&self, uri: &LspUri) -> DocModule {
        if let LspUri::File(path) = uri {
            if crate::bazel::is_bazel_file(path) {
                return crate::bazel::environment_for(path).clone();
            }
            if path.file_name().and_then(|n| n.to_str()) == Some("MODULE.aspect") {
                return axl_runtime::module::manifest_globals().documentation();
            }
            if path.file_name().and_then(|n| n.to_str())
                == Some(axl_runtime::module::AXL_VERSION_EXTENSION)
            {
                static VERSION_ENV: std::sync::OnceLock<DocModule> = std::sync::OnceLock::new();
                return VERSION_ENV
                    .get_or_init(|| {
                        let mut doc = starlark::environment::GlobalsBuilder::standard()
                            .build()
                            .documentation();
                        doc.members.insert(
                            "version".to_owned(),
                            starlark::docs::DocItem::Member(starlark::docs::DocMember::Function(
                                starlark::docs::DocFunction {
                                    docs: Some(starlark::docs::DocString {
                                        summary: "Pins the aspect-cli version for this workspace."
                                            .to_owned(),
                                        ..Default::default()
                                    }),
                                    ..Default::default()
                                },
                            )),
                        );
                        doc
                    })
                    .clone();
            }
        }
        eval::get_globals().build().documentation()
    }

    fn get_uri_for_global_symbol(
        &self,
        _current_file: &LspUri,
        _symbol: &str,
    ) -> Result<Option<LspUri>, String> {
        Ok(None)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn diagnostics(file_name: &str, content: &str) -> Vec<String> {
        let ctx = AxlContext {};
        let uri = LspUri::File(PathBuf::from(file_name));
        ctx.parse_file_with_contents(&uri, content.to_owned())
            .diagnostics
            .into_iter()
            .map(|d| d.message)
            .collect()
    }

    #[test]
    fn undefined_symbol_is_reported() {
        let d = diagnostics(
            "/w/postgres.bzl",
            "def _impl(ctx):\n    return [AppInfo(app = 1)]\n\nr = rule(implementation = _impl, provides = [AppInfo])\n",
        );
        assert!(
            d.iter()
                .any(|m| m.contains("undefined") && m.contains("AppInfo")),
            "{d:?}"
        );
    }

    #[test]
    fn loaded_symbol_resolves() {
        let d = diagnostics(
            "/w/ok.bzl",
            "load(\":p.bzl\", \"AppInfo\")\n\ndef _impl(ctx):\n    return [AppInfo(app = 1)]\n\nr = rule(implementation = _impl, provides = [AppInfo])\n",
        );
        assert!(d.is_empty(), "{d:?}");
    }

    #[test]
    fn builtins_and_locals_do_not_warn() {
        let d = diagnostics(
            "/w/clean.bzl",
            "def _impl(ctx):\n    outs = [f for f in ctx.files.srcs]\n    return [DefaultInfo(files = depset(outs))]\n",
        );
        assert!(d.is_empty(), "{d:?}");
    }
}

#[cfg(test)]
mod sweep {
    use super::*;

    #[test]
    fn sweep_local_repo_for_false_positives() {
        let ctx = AxlContext {};
        let mut findings = 0;
        let mut files = 0;
        let mut stack: Vec<PathBuf> = [
            "/Users/abel/code/rules_py",
            "/Users/abel/code/cosmos_aspect-cli/.aspect",
        ]
        .iter()
        .map(PathBuf::from)
        .filter(|p| p.is_dir())
        .collect();
        if stack.is_empty() {
            return;
        }
        while let Some(dir) = stack.pop() {
            let Ok(entries) = std::fs::read_dir(&dir) else {
                continue;
            };
            for entry in entries.flatten() {
                let path = entry.path();
                let name = entry.file_name().to_string_lossy().to_string();
                if path.is_dir() {
                    if !name.starts_with("bazel-") && name != ".git" && name != "node_modules" {
                        stack.push(path);
                    }
                    continue;
                }
                let is_starlark = name.ends_with(".bzl")
                    || name.ends_with(".axl")
                    || name == "BUILD.bazel"
                    || name == "BUILD"
                    || name == "MODULE.aspect"
                    || name.ends_with("MODULE.bazel");
                if !is_starlark {
                    continue;
                }
                let Ok(content) = std::fs::read_to_string(&path) else {
                    continue;
                };
                files += 1;
                let result = ctx.parse_file_with_contents(&LspUri::File(path.clone()), content);
                for d in result.diagnostics {
                    findings += 1;
                    println!(
                        "SWEEP {}:{} {}",
                        path.display(),
                        d.range.start.line + 1,
                        d.message
                    );
                }
            }
        }
        println!("SWEEP total: {files} files, {findings} findings");
    }
}

#[cfg(test)]
mod axl_env_tests {
    use super::*;

    fn diagnostics(file_name: &str, content: &str) -> Vec<String> {
        let ctx = AxlContext {};
        ctx.parse_file_with_contents(&LspUri::File(PathBuf::from(file_name)), content.to_owned())
            .diagnostics
            .into_iter()
            .map(|d| d.message)
            .collect()
    }

    #[test]
    fn axl_undefined_symbol_is_reported() {
        let d = diagnostics(
            "/w/status.axl",
            "def _impl(ctx):\n    return SomethingUndeclared(x = 1)\n\nstatus = task(implementation = _impl)\n",
        );
        assert!(d.iter().any(|m| m.contains("SomethingUndeclared")), "{d:?}");
    }

    #[test]
    fn axl_builtins_do_not_warn() {
        let d = diagnostics(
            "/w/clean.axl",
            "T = trait(hook = attr(list, default = []))\n\ndef _impl(ctx):\n    print(json.encode({}))\n    return 0\n\nt = task(implementation = _impl, traits = [T])\n",
        );
        assert!(d.is_empty(), "{d:?}");
    }

    #[test]
    fn module_aspect_uses_manifest_globals() {
        let d = diagnostics(
            "/w/MODULE.aspect",
            "axl_dep(name = \"mattermost\", version = \"1.0.0\")\nuse_task(\"@mattermost//task/notify.axl\", \"notify\")\n",
        );
        assert!(d.is_empty(), "{d:?}");
        let bad = diagnostics("/w/MODULE.aspect", "t = task(implementation = None)\n");
        assert!(
            bad.iter().any(|m| m.contains(
                "`task` is not available in MODULE.aspect: it can only be used in .axl module files"
            )),
            "{bad:?}"
        );
        let inv = diagnostics("/w/notify.axl", "axl_dep(name = \"x\", version = \"1\")\n");
        assert!(
            inv.iter().any(|m| m.contains(
                "`axl_dep` is not available in .axl files: it can only be used in MODULE.aspect"
            )),
            "{inv:?}"
        );
    }
}

#[cfg(test)]
mod file_kind_scoping_tests {
    use super::*;

    fn diagnostics(file_name: &str, content: &str) -> Vec<String> {
        let ctx = AxlContext {};
        ctx.parse_file_with_contents(&LspUri::File(PathBuf::from(file_name)), content.to_owned())
            .diagnostics
            .into_iter()
            .map(|d| d.message)
            .collect()
    }

    #[test]
    fn module_directives_are_errors_in_bzl() {
        let d = diagnostics(
            "/w/ingress.bzl",
            "load(\":app.bzl\", \"AppInfo\")\nbazel_dep(name = \"aspect_bazel_lib\", version = \"2.22.0\")\n",
        );
        assert!(
            d.iter().any(|m| m
                .contains("`bazel_dep` is not available in .bzl files: it can only be used in MODULE.bazel files")),
            "{d:?}"
        );
    }

    #[test]
    fn build_natives_are_errors_in_bzl() {
        let d = diagnostics("/w/defs.bzl", "srcs = glob([\"*.py\"])\n");
        assert!(
            d.iter()
                .any(|m| m.contains("can only be used in BUILD files")
                    && m.contains("call it as `native.glob`")),
            "{d:?}"
        );
        let ok = diagnostics(
            "/w/defs.bzl",
            "def m():\n    return native.glob([\"*.py\"])\n",
        );
        assert!(ok.is_empty(), "{ok:?}");
    }

    #[test]
    fn build_files_accept_natives_but_not_bzl_api() {
        let ok = diagnostics(
            "/w/BUILD.bazel",
            "genrule(name = \"g\", outs = [\"o\"], cmd = \"touch $@\")\nfilegroup(name = \"f\", srcs = glob([\"*.txt\"]))\n",
        );
        assert!(ok.is_empty(), "{ok:?}");
        let bad = diagnostics("/w/BUILD.bazel", "r = rule(implementation = None)\n");
        assert!(bad.iter().any(|m| m.contains("rule")), "{bad:?}");
    }

    #[test]
    fn module_bazel_accepts_directives_but_not_bzl_api() {
        let ok = diagnostics(
            "/w/MODULE.bazel",
            "module(name = \"x\")\nbazel_dep(name = \"rules_cc\", version = \"0.2.16\")\nuv = use_extension(\"//:e.bzl\", \"uv\")\nuse_repo(uv, \"pypi\")\n",
        );
        assert!(ok.is_empty(), "{ok:?}");
        let bad = diagnostics("/w/MODULE.bazel", "r = rule(implementation = None)\n");
        assert!(bad.iter().any(|m| m.contains("rule")), "{bad:?}");
    }
}
