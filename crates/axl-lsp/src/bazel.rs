use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Output, Stdio};
use std::sync::{Mutex, OnceLock};
use std::time::{Duration, Instant};

use starlark::docs::{DocFunction, DocItem, DocMember, DocModule, DocString};
use starlark::environment::GlobalsBuilder;

const BAZEL_FILE_NAMES: &[&str] = &[
    "BUILD",
    "BUILD.bazel",
    "MODULE.bazel",
    "WORKSPACE",
    "WORKSPACE.bazel",
    "WORKSPACE.bzlmod",
    "REPO.bazel",
    "VENDOR.bazel",
];

const WORKSPACE_BOUNDARY_FILES: &[&str] =
    &["MODULE.bazel", "WORKSPACE", "WORKSPACE.bazel", "REPO.bazel"];

pub fn is_bazel_file(path: &Path) -> bool {
    let Some(name) = path.file_name().and_then(|n| n.to_str()) else {
        return false;
    };
    name.ends_with(".bzl")
        || name.ends_with(".star")
        || name.ends_with(".bazel")
        || BAZEL_FILE_NAMES.contains(&name)
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum FileKind {
    Bzl,
    Build,
    Module,
    Workspace,
}

pub fn file_kind(path: &Path) -> Option<FileKind> {
    let name = path.file_name().and_then(|n| n.to_str())?;
    if name.ends_with(".bzl") || name.ends_with(".star") {
        return Some(FileKind::Bzl);
    }
    if name == "BUILD" || name == "BUILD.bazel" {
        return Some(FileKind::Build);
    }
    if name == "MODULE.bazel" || name.ends_with(".MODULE.bazel") {
        return Some(FileKind::Module);
    }
    if name.starts_with("WORKSPACE") {
        return Some(FileKind::Workspace);
    }
    if name == "REPO.bazel" || name == "VENDOR.bazel" {
        return Some(FileKind::Module);
    }
    if name.ends_with(".bazel") {
        return Some(FileKind::Build);
    }
    None
}

pub fn is_bazel_label(path: &str) -> bool {
    path.contains(':') || path.starts_with("//")
}

pub fn resolve_label(label: &str, current_dir: &Path) -> Result<PathBuf, String> {
    let candidates = label_candidates(label, current_dir)?;
    candidates
        .iter()
        .find(|c| c.is_file())
        .cloned()
        .ok_or_else(|| {
            format!(
                "cannot resolve label \"{label}\": tried {}",
                candidates
                    .iter()
                    .map(|c| c.display().to_string())
                    .collect::<Vec<_>>()
                    .join(", ")
            )
        })
}

pub fn best_candidate(label: &str, current_dir: &Path) -> Option<PathBuf> {
    label_candidates(label, current_dir)
        .ok()?
        .into_iter()
        .next()
}

fn label_candidates(label: &str, current_dir: &Path) -> Result<Vec<PathBuf>, String> {
    if let Some(rest) = label.strip_prefix(':') {
        return Ok(vec![current_dir.join(rest)]);
    }

    let (repo, rest) =
        if let Some(after) = label.strip_prefix("@@").or_else(|| label.strip_prefix('@')) {
            let (repo, rest) = after
                .split_once("//")
                .ok_or_else(|| format!("malformed label: {label}"))?;
            (Some(repo), rest)
        } else if let Some(rest) = label.strip_prefix("//") {
            (None, rest)
        } else {
            return Err(format!("not a bazel label: {label}"));
        };

    let rel: PathBuf = match rest.split_once(':') {
        Some((pkg, target)) => Path::new(pkg).join(target),
        None => PathBuf::from(rest),
    };

    let workspace = nearest_workspace_root(current_dir)
        .ok_or_else(|| format!("no MODULE.bazel/WORKSPACE above {}", current_dir.display()))?;

    match repo {
        None => Ok(vec![workspace.join(rel)]),
        Some(repo) => {
            let mut cands = Vec::new();
            if let Ok(module_text) = std::fs::read_to_string(workspace.join("MODULE.bazel")) {
                if module_name(&module_text).as_deref() == Some(repo) {
                    cands.push(workspace.join(&rel));
                }
                if let Some(path) = local_path_override(&module_text, repo) {
                    cands.push(workspace.join(path).join(&rel));
                }
            }
            match external_dir(&workspace) {
                Some(external) => {
                    cands.push(external.join(repo).join(&rel));
                    if let Ok(entries) = std::fs::read_dir(&external) {
                        for entry in entries.flatten() {
                            let name = entry.file_name();
                            let name = name.to_string_lossy();
                            if name.starts_with(&format!("{repo}+"))
                                || name.starts_with(&format!("{repo}~"))
                            {
                                cands.push(entry.path().join(&rel));
                            }
                        }
                    }
                }
                None if cands.is_empty() => {
                    return Err(
                        "cannot locate <output_base>/external: no bazel-* symlinks and \
                         `bazel info output_base` failed (is bazel on PATH?)"
                            .to_owned(),
                    );
                }
                None => {}
            }
            Ok(cands)
        }
    }
}

fn module_name(module_text: &str) -> Option<String> {
    let idx = module_text.find("module(")?;
    let tail = &module_text[idx..module_text.len().min(idx + 500)];
    let name_idx = tail.find("name")?;
    let rest = &tail[name_idx..];
    let quote = rest.find('"')? + 1;
    let end = rest[quote..].find('"')? + quote;
    Some(rest[quote..end].to_owned())
}

fn local_path_override(module_text: &str, repo: &str) -> Option<String> {
    let needle = format!("module_name = \"{repo}\"");
    let mut search = module_text;
    loop {
        let idx = search.find("local_path_override")?;
        let block_end = search[idx..].find(')')? + idx;
        let block = &search[idx..block_end];
        if block.contains(&needle) {
            let path_idx = block.find("path")?;
            let rest = &block[path_idx..];
            let quote = rest.find('"')? + 1;
            let close = rest[quote..].find('"')? + quote;
            return Some(rest[quote..close].to_owned());
        }
        search = &search[block_end + 1..];
    }
}

fn nearest_workspace_root(from: &Path) -> Option<PathBuf> {
    let mut dir = Some(from);
    while let Some(d) = dir {
        if WORKSPACE_BOUNDARY_FILES.iter().any(|f| d.join(f).is_file()) {
            return Some(d.to_path_buf());
        }
        dir = d.parent();
    }
    None
}

const OUTPUT_BASE_TIMEOUT: Duration = Duration::from_secs(5);
const OUTPUT_BASE_POLL: Duration = Duration::from_millis(25);

fn external_dir(workspace: &Path) -> Option<PathBuf> {
    external_dir_with_fallback(workspace, bazel_output_base)
}

fn external_dir_with_fallback(
    workspace: &Path,
    fallback: impl Fn(&Path) -> Option<PathBuf>,
) -> Option<PathBuf> {
    if let Some(dir) = external_dir_via_symlinks(workspace) {
        return Some(dir);
    }
    let external = fallback(workspace)?.join("external");
    external.is_dir().then_some(external)
}

fn external_dir_via_symlinks(workspace: &Path) -> Option<PathBuf> {
    let entries = std::fs::read_dir(workspace).ok()?;
    for entry in entries.flatten() {
        let name = entry.file_name();
        if !name.to_string_lossy().starts_with("bazel-") {
            continue;
        }
        let Ok(target) = std::fs::read_link(entry.path()) else {
            continue;
        };
        let mut dir = target.as_path();
        while let Some(parent) = dir.parent() {
            if dir.file_name().is_some_and(|n| n == "execroot") {
                let external = parent.join("external");
                if external.is_dir() {
                    return Some(external);
                }
            }
            dir = parent;
        }
    }
    None
}

fn bazel_output_base(workspace: &Path) -> Option<PathBuf> {
    static CACHE: OnceLock<Mutex<HashMap<PathBuf, PathBuf>>> = OnceLock::new();
    let cache = CACHE.get_or_init(|| Mutex::new(HashMap::new()));

    if let Ok(Some(base)) = cache.lock().map(|m| m.get(workspace).cloned()) {
        return Some(base);
    }

    let bazel = std::env::var_os("BAZEL").unwrap_or_else(|| "bazel".into());
    let child = Command::new(bazel)
        .args(["info", "output_base"])
        .current_dir(workspace)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .ok()?;
    let output = wait_with_deadline(child, OUTPUT_BASE_TIMEOUT)?;
    let base = PathBuf::from(String::from_utf8_lossy(&output.stdout).trim());
    if base.as_os_str().is_empty() {
        return None;
    }
    if let Ok(mut map) = cache.lock() {
        map.insert(workspace.to_path_buf(), base.clone());
    }
    Some(base)
}

fn wait_with_deadline(mut child: Child, timeout: Duration) -> Option<Output> {
    let deadline = Instant::now() + timeout;
    loop {
        match child.try_wait() {
            Ok(Some(_)) => return child.wait_with_output().ok(),
            Ok(None) if Instant::now() < deadline => std::thread::sleep(OUTPUT_BASE_POLL),
            _ => {
                let _ = child.kill();
                let _ = child.wait();
                return None;
            }
        }
    }
}

#[cfg(test)]
mod external_dir_tests {
    use super::*;

    fn temp_workspace(tag: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("axl-lsp-extdir-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[cfg(unix)]
    #[test]
    fn symlinks_present_short_circuit_without_bazel() {
        let ws = temp_workspace("symlink");
        let output_base = ws.join("out");
        let execroot = output_base.join("execroot").join("wsname");
        std::fs::create_dir_all(&execroot).unwrap();
        std::fs::create_dir_all(output_base.join("external")).unwrap();
        std::os::unix::fs::symlink(&execroot, ws.join("bazel-wsname")).unwrap();

        let got = external_dir_with_fallback(&ws, |_| {
            panic!("no debe spawnearse bazel si los symlinks resuelven")
        });
        assert_eq!(got, Some(output_base.join("external")));
        let _ = std::fs::remove_dir_all(&ws);
    }

    #[test]
    fn sin_symlinks_cae_al_fallback_de_output_base() {
        let ws = temp_workspace("fallback");
        std::fs::write(ws.join("MODULE.bazel"), "module(name = \"wsname\")\n").unwrap();
        let output_base = ws.join("out");
        std::fs::create_dir_all(output_base.join("external")).unwrap();

        let got = external_dir_with_fallback(&ws, |_| Some(output_base.clone()));
        assert_eq!(got, Some(output_base.join("external")));

        let missing = external_dir_with_fallback(&ws, |_| Some(ws.join("nope")));
        assert_eq!(missing, None);
        let _ = std::fs::remove_dir_all(&ws);
    }

    #[cfg(unix)]
    #[test]
    fn repo_no_fetcheado_degrada_a_candidato_canonico() {
        let ws = temp_workspace("unfetched");
        std::fs::write(ws.join("MODULE.bazel"), "module(name = \"wsname\")\n").unwrap();
        let output_base = ws.join("out");
        let execroot = output_base.join("execroot").join("wsname");
        std::fs::create_dir_all(&execroot).unwrap();
        std::fs::create_dir_all(output_base.join("external")).unwrap();
        std::os::unix::fs::symlink(&execroot, ws.join("bazel-wsname")).unwrap();

        let label = "@demo//pkg:lib.bzl";
        assert!(resolve_label(label, &ws).is_err());
        assert_eq!(
            best_candidate(label, &ws),
            Some(
                output_base
                    .join("external")
                    .join("demo")
                    .join("pkg")
                    .join("lib.bzl")
            ),
        );
        let _ = std::fs::remove_dir_all(&ws);
    }
}

const BAZEL_BUILTINS: &[(&str, &str, &str)] = &[
    (
        "rule",
        "Defines a new rule; returns a callable registered in BUILD files.",
        "Main parameters: `implementation` (function taking ctx), `attrs` (dict of attr.* schemas), \
         `executable`/`test` (impl must return DefaultInfo(executable=...)), `toolchains`, \
         `fragments`, `provides`.\n\nDocs: https://bazel.build/rules/lib/globals/bzl#rule",
    ),
    (
        "attr",
        "Module of attribute schemas for rule/aspect/repository_rule definitions.",
        "Primitives: `bool`, `int`, `string` (restrict with `values=[...]`).\n\
         Lists: `int_list`, `string_list`.\n\
         Dependencies: `label`, `label_list` — key params: `providers=[...]` (required providers), \
         `cfg=\"exec\"|\"target\"`, `allow_files`/`allow_single_file`, `executable`, `aspects`, `mandatory`.\n\
         Dicts: `string_dict`, `string_list_dict`, `label_keyed_string_dict`, `string_keyed_label_dict`, `label_list_dict`.\n\
         Caller-named outputs: `output`, `output_list`.\n\n\
         Docs: https://bazel.build/rules/lib/toplevel/attr",
    ),
    (
        "provider",
        "Declares a provider symbol carrying information between rules.",
        "",
    ),
    (
        "aspect",
        "Defines an aspect that traverses dependency edges of a target.",
        "",
    ),
    (
        "depset",
        "Creates an immutable transitive set optimized for merging.",
        "",
    ),
    (
        "Label",
        "Parses a label string into a canonical Label object.",
        "",
    ),
    (
        "struct",
        "Creates an immutable struct with the given fields.",
        "",
    ),
    (
        "glob",
        "Expands file patterns relative to the current package (BUILD only).",
        "`glob(include, exclude=[], exclude_directories=1, allow_empty=?)` — `include`: patterns \
         (`**` crosses directories, never package boundaries); `exclude`: filtered out; \
         `allow_empty=False` errors when nothing matches (recommended).",
    ),
    (
        "select",
        "Configurable attribute value keyed by config_setting labels.",
        "`select({\"//cond:a\": v1, \"//conditions:default\": v2}, no_match_error=?)` — keys are \
         `config_setting`/`constraint_value` labels; exactly one condition wins per invocation.",
    ),
    (
        "native",
        "Namespace exposing native rules from .bzl files (native.glob, native.existing_rules...).",
        "",
    ),
    (
        "package",
        "Sets package-level defaults such as default_visibility (BUILD only).",
        "Main args: `default_visibility=[...]`, `default_testonly`, `default_deprecation`, `features`.",
    ),
    (
        "exports_files",
        "Makes source files visible to other packages (BUILD only).",
        "`exports_files(srcs, visibility=?, licenses=?)` — `srcs`: list of files in this package to \
         export; `visibility`: who can use them (default: everyone); needed so other packages can \
         reference plain files like `//pkg:file.tpl` in their rules.",
    ),
    (
        "filegroup",
        "Groups targets/files under one label (BUILD only).",
        "Main attrs: `srcs` (targets whose outputs are gathered), `data` (runtime files added to \
         runfiles of dependers), `output_group` (which output group to re-export).",
    ),
    (
        "alias",
        "Gives another name to a target (BUILD only).",
        "`alias(name, actual)` — `actual` is the referenced target; can also point to an input file.",
    ),
    (
        "genrule",
        "Runs a Bash command producing declared outputs (BUILD only).",
        "Main attrs: `srcs` (inputs, `$(SRCS)`), `outs` (declared outputs, `$(OUTS)`/`$@`), \
         `cmd`/`cmd_bash` (command with make-var expansion, `$(location //t)`), `tools` (exec-config \
         binaries), `executable`, `local`, `message`.",
    ),
    (
        "config_setting",
        "Matches build flags/platform for use as select() key (BUILD only).",
        "Main attrs: `values` (flag=value dict), `define_values` (--define pairs), \
         `constraint_values` (platform constraints), `flag_values` (user-defined build settings).",
    ),
    (
        "test_suite",
        "Groups tests under one label (BUILD only).",
        "`tests`: explicit test targets (default: all non-manual tests in the package); \
         `tags`: filter, `-tag` excludes.",
    ),
    (
        "package_group",
        "Named set of packages for visibility control (BUILD only).",
        "`packages`: specs like `//foo/...`; `includes`: other package_groups composed transitively.",
    ),
    (
        "existing_rules",
        "Returns a snapshot of the rules declared so far in the package.",
        "",
    ),
    (
        "module_extension",
        "Declares a bzlmod module extension (MODULE.bazel ecosystem).",
        "",
    ),
    (
        "tag_class",
        "Declares a tag class consumed by a module_extension.",
        "",
    ),
    (
        "repository_rule",
        "Defines a repository rule executed during the fetch phase.",
        "",
    ),
    (
        "bazel_dep",
        "Declares a dependency on another Bazel module (MODULE.bazel only).",
        "",
    ),
    (
        "use_extension",
        "Brings a module extension into scope (MODULE.bazel only).",
        "",
    ),
    (
        "use_repo",
        "Imports repos generated by a module extension (MODULE.bazel only).",
        "",
    ),
    (
        "use_repo_rule",
        "Imports a repository rule for direct use (MODULE.bazel only).",
        "",
    ),
    (
        "register_toolchains",
        "Registers toolchain targets for resolution.",
        "",
    ),
    (
        "register_execution_platforms",
        "Registers execution platforms for resolution.",
        "",
    ),
    (
        "module",
        "Declares the current module's name and version (MODULE.bazel only).",
        "",
    ),
    (
        "visibility",
        "Sets the load-visibility of the current .bzl file.",
        "",
    ),
    (
        "config_common",
        "Helpers for user-defined build settings.",
        "",
    ),
    ("json", "JSON encode/decode module.", ""),
    ("proto", "Textproto encode module.", ""),
    ("testing", "Helpers for analysis-test rules.", ""),
];

const BAZEL_EXTRA_GLOBALS: &[(&str, &str)] = &[
    (
        "DefaultInfo",
        "Provider with a target's default outputs and runfiles.",
    ),
    (
        "OutputGroupInfo",
        "Provider mapping output group names to depsets.",
    ),
    (
        "RunEnvironmentInfo",
        "Provider with env for executable targets.",
    ),
    (
        "InstrumentedFilesInfo",
        "Provider with coverage-instrumented sources.",
    ),
    (
        "PackageSpecificationInfo",
        "package_group specification provider.",
    ),
    (
        "config",
        "Build setting type constructors for rule(build_setting=...).",
    ),
    (
        "configuration_field",
        "Late-bound default for label attributes.",
    ),
    ("coverage_common", "Coverage instrumentation utilities."),
    (
        "platform_common",
        "ToolchainInfo/TemplateVariableInfo constructors.",
    ),
    (
        "analysis_test_transition",
        "Configuration transition for analysis tests.",
    ),
    (
        "transition",
        "Starlark configuration transition constructor.",
    ),
    (
        "exec_group",
        "Execution group constructor for rule(exec_groups=...).",
    ),
    ("subrule", "Subrule constructor (Bazel 7+)."),
    ("macro", "Symbolic macro constructor (Bazel 8+)."),
    (
        "toolchain",
        "Registers a toolchain implementation for a toolchain type.",
    ),
    ("toolchain_type", "Declares a toolchain type label."),
    ("platform", "Defines a platform from constraint values."),
    ("constraint_setting", "Declares a constraint dimension."),
    (
        "constraint_value",
        "Declares a value for a constraint_setting.",
    ),
    ("label_flag", "Label-typed build flag."),
    ("label_setting", "Label-typed non-flag build setting."),
    ("licenses", "Legacy license declaration."),
    ("environment", "Legacy environment target."),
    ("environment_group", "Legacy environment group."),
    ("package_name", "Name of the package being evaluated."),
    (
        "repository_name",
        "Canonical name of the current repository.",
    ),
    (
        "archive_override",
        "MODULE.bazel: take a dep from an archive URL.",
    ),
    (
        "git_override",
        "MODULE.bazel: take a dep from a git commit.",
    ),
    (
        "local_path_override",
        "MODULE.bazel: take a dep from a local directory.",
    ),
    (
        "single_version_override",
        "MODULE.bazel: pin or patch a dep version.",
    ),
    (
        "multiple_version_override",
        "MODULE.bazel: allow several dep versions.",
    ),
    (
        "include",
        "MODULE.bazel: textually include a *.MODULE.bazel segment.",
    ),
    (
        "inject_repo",
        "MODULE.bazel: inject repos into an extension.",
    ),
    (
        "override_repo",
        "MODULE.bazel: override an extension's repos.",
    ),
    ("workspace", "WORKSPACE: declares the workspace name."),
    (
        "local_repository",
        "WORKSPACE: local directory as external repo.",
    ),
    (
        "new_local_repository",
        "WORKSPACE: local directory with injected BUILD.",
    ),
    (
        "print",
        "Prints a debug message (shows as DEBUG in the Bazel console).",
    ),
    (
        "genquery",
        "Runs a query over the target's dependency graph at build time.",
    ),
    ("set", "Mutable set type (Bazel 8+)."),
    (
        "AnalysisTestResultInfo",
        "Provider returned by analysis tests.",
    ),
    (
        "providers",
        "cquery --output=starlark helper: providers of a target.",
    ),
];

const MODULE_ONLY: &[&str] = &[
    "module",
    "bazel_dep",
    "use_extension",
    "use_repo",
    "use_repo_rule",
    "archive_override",
    "git_override",
    "local_path_override",
    "single_version_override",
    "multiple_version_override",
    "include",
    "inject_repo",
    "override_repo",
];

const WORKSPACE_ONLY: &[&str] = &["workspace", "local_repository", "new_local_repository"];

const MODULE_OR_WORKSPACE: &[&str] = &["register_toolchains", "register_execution_platforms"];

const BUILD_ONLY: &[&str] = &[
    "genrule",
    "genquery",
    "filegroup",
    "alias",
    "config_setting",
    "test_suite",
    "package_group",
    "exports_files",
    "glob",
    "package",
    "existing_rules",
    "toolchain",
    "toolchain_type",
    "platform",
    "constraint_setting",
    "constraint_value",
    "label_flag",
    "label_setting",
    "licenses",
    "environment",
    "environment_group",
    "package_name",
    "repository_name",
];

const SHARED_BZL_BUILD: &[&str] = &[
    "select", "depset", "Label", "struct", "json", "set", "print",
];

fn allowed_in(kind: FileKind, name: &str) -> bool {
    if MODULE_ONLY.contains(&name) {
        return kind == FileKind::Module;
    }
    if WORKSPACE_ONLY.contains(&name) {
        return kind == FileKind::Workspace;
    }
    if MODULE_OR_WORKSPACE.contains(&name) {
        return matches!(kind, FileKind::Module | FileKind::Workspace);
    }
    if name == "print" {
        return true;
    }
    if BUILD_ONLY.contains(&name) {
        return kind == FileKind::Build;
    }
    if SHARED_BZL_BUILD.contains(&name) {
        return matches!(kind, FileKind::Bzl | FileKind::Build);
    }
    kind == FileKind::Bzl
}

pub fn kind_description(path: &Path) -> &'static str {
    match file_kind(path) {
        Some(FileKind::Bzl) | None => ".bzl files",
        Some(FileKind::Build) => "BUILD files",
        Some(FileKind::Module) => "MODULE.bazel",
        Some(FileKind::Workspace) => "WORKSPACE",
    }
}

pub fn is_build_native(name: &str) -> bool {
    BUILD_ONLY.contains(&name)
}

pub fn available_in(name: &str) -> Option<&'static str> {
    if MODULE_ONLY.contains(&name) {
        return Some("MODULE.bazel files");
    }
    if WORKSPACE_ONLY.contains(&name) {
        return Some("WORKSPACE files");
    }
    if MODULE_OR_WORKSPACE.contains(&name) {
        return Some("MODULE.bazel and WORKSPACE files");
    }
    if BUILD_ONLY.contains(&name) {
        return Some("BUILD files");
    }
    if SHARED_BZL_BUILD.contains(&name) {
        return Some(".bzl and BUILD files");
    }
    if BAZEL_BUILTINS.iter().any(|(n, _, _)| *n == name)
        || BAZEL_EXTRA_GLOBALS.iter().any(|(n, _)| *n == name)
    {
        return Some(".bzl files");
    }
    None
}

fn build_environment(kind: FileKind) -> DocModule {
    let mut doc = GlobalsBuilder::standard().build().documentation();
    for (name, summary) in BAZEL_EXTRA_GLOBALS {
        if !allowed_in(kind, name) {
            continue;
        }
        doc.members.insert(
            (*name).to_owned(),
            DocItem::Member(DocMember::Function(DocFunction {
                docs: Some(DocString {
                    summary: (*summary).to_owned(),
                    ..Default::default()
                }),
                ..Default::default()
            })),
        );
    }
    for (name, summary, details) in BAZEL_BUILTINS {
        if !allowed_in(kind, name) {
            continue;
        }
        doc.members.insert(
            (*name).to_owned(),
            DocItem::Member(DocMember::Function(DocFunction {
                docs: Some(DocString {
                    summary: (*summary).to_owned(),
                    details: if details.is_empty() {
                        None
                    } else {
                        Some((*details).to_owned())
                    },
                    ..Default::default()
                }),
                ..Default::default()
            })),
        );
    }
    doc
}

pub fn environment_for(path: &Path) -> &'static DocModule {
    static BZL: OnceLock<DocModule> = OnceLock::new();
    static BUILD: OnceLock<DocModule> = OnceLock::new();
    static MODULE: OnceLock<DocModule> = OnceLock::new();
    static WORKSPACE: OnceLock<DocModule> = OnceLock::new();
    match file_kind(path).unwrap_or(FileKind::Bzl) {
        FileKind::Bzl => BZL.get_or_init(|| build_environment(FileKind::Bzl)),
        FileKind::Build => BUILD.get_or_init(|| build_environment(FileKind::Build)),
        FileKind::Module => MODULE.get_or_init(|| build_environment(FileKind::Module)),
        FileKind::Workspace => WORKSPACE.get_or_init(|| build_environment(FileKind::Workspace)),
    }
}
