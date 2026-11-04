//! Label normalization for matching what a task asked to run against what
//! BES reports.
//!
//! [`normalize_label`] maps every spelling of a target to one key. It mirrors
//! `normalize_label` in `crates/aspect-cli/src/builtins/aspect/private/lib/runnable.axl`,
//! which the AXL tasks use for the same matching; the two must agree on every
//! label; both test suites run the cases in `label_cases.json`.
//! [`resolve_relative_label`] turns a label written relative to a working
//! directory into an absolute one first, the way Bazel's command line does,
//! and [`LabelResolver`] does both against a workspace on disk.

use std::path::{Path, PathBuf};

/// Return the standard spelling of a Bazel label: as written in this
/// workspace, with the target name spelled out. Two labels name the same
/// target exactly when their normalized forms are equal.
///
/// Repositories are named by their *apparent* name, the short name a
/// `MODULE.bazel` gives a dependency (`@rules_go`), and the main repository is
/// written `//`. Bazel often reports *canonical* names instead: the unique
/// `@@` name it assigns each repository, which can carry a version or an
/// extension (`@@rules_go+`, `@@rules_python++pip+numpy`). See
/// <https://bazel.build/external/overview#apparent-repo-name>,
/// <https://bazel.build/external/overview#canonical-repo-name> and
/// <https://bazel.build/concepts/labels>.
///
/// - `@@//pkg:t` and `@//pkg:t` → `//pkg:t`
/// - module repo `@@module+version//...` or `@@module~//...` → `@module//...`
/// - extension repo `@@module++ext+repo//...` or `@@module~~ext~repo//...`
///   → `@repo//...`
/// - a bare repository, `@repo` or `@repo//` → `@repo//:repo`
///
/// The apparent name is derived from the canonical one's shape, not from
/// Bazel's repository mapping, so a dependency declared with a different
/// `repo_name` comes back under its module name.
///
/// A label with no `:` gets its implicit target name: the last package
/// segment, or the repository name for a bare repository label.
///
/// A relative label (`:tool`, `tool`, `pkg:tool`) is read as relative to the
/// root package, as a BUILD file there would read it: `tool` is `//:tool`.
/// From another working directory its meaning depends on which directories
/// are packages, which this function cannot see; resolve it first with
/// [`LabelResolver`].
pub fn normalize_label(label: &str) -> String {
    let mut label = label.to_owned();
    if label.starts_with('@') && !label.contains("//") {
        label.push_str("//");
    }
    if let Some(rest) = label.strip_prefix("@@").map(str::to_owned)
        && let Some(idx) = rest.find("//")
    {
        let canonical_repo = &rest[..idx];
        if canonical_repo.is_empty() {
            label = rest[idx..].to_owned();
        } else {
            // `~` (Bazel 7) never appears in a version; `+` can, in semver
            // build metadata.
            let sep = if canonical_repo.contains('~') {
                '~'
            } else {
                '+'
            };
            let parts: Vec<&str> = canonical_repo.split(sep).collect();
            let apparent_repo = if parts.len() >= 3 {
                parts[parts.len() - 1]
            } else {
                parts[0]
            };
            label = format!("@{apparent_repo}{}", &rest[idx..]);
        }
    }
    if let Some(rest) = label.strip_prefix("@//").map(str::to_owned) {
        label = format!("//{rest}");
    }

    if !label.starts_with("//") && !label.starts_with('@') {
        if label.contains(':') {
            return format!("//{label}");
        }
        return format!("//:{label}");
    }

    if label.contains(':') {
        return label;
    }
    let (repo, path) = match label.find("//") {
        Some(idx) => (&label[..idx], &label[idx + 2..]),
        None => return label,
    };
    if path.is_empty() {
        let name = repo.trim_start_matches('@');
        return format!("{repo}//:{name}");
    }
    let name = path.rsplit('/').next().unwrap_or(path);
    format!("{repo}//{path}:{name}")
}

/// Resolves labels the way Bazel reads them on the command line: relative to
/// the directory it was invoked from, in one workspace.
#[derive(Debug, Clone)]
pub struct LabelResolver {
    workspace_root: PathBuf,
    current_package: String,
}

impl LabelResolver {
    /// A resolver for a command run from `working_dir`. Outside the
    /// workspace, relative labels resolve against the root package.
    pub fn new(workspace_root: &Path, working_dir: &Path) -> Self {
        let canonical = |p: &Path| p.canonicalize().unwrap_or_else(|_| p.to_owned());
        let workspace_root = canonical(workspace_root);
        let current_package = match canonical(working_dir).strip_prefix(&workspace_root) {
            Ok(rel) => rel.to_string_lossy().replace('\\', "/"),
            Err(_) => String::new(),
        };
        Self {
            workspace_root,
            current_package,
        }
    }

    /// The workspace-relative directory relative labels resolve against,
    /// `""` for the root.
    pub fn current_package(&self) -> &str {
        &self.current_package
    }

    /// [`resolve_relative_label`] against this workspace's packages.
    pub fn resolve(&self, label: &str) -> String {
        resolve_relative_label(label, &self.current_package, |pkg| {
            is_package(&self.workspace_root, pkg)
        })
    }

    /// [`normalize_label`] of [`Self::resolve`].
    pub fn normalize(&self, label: &str) -> String {
        normalize_label(&self.resolve(label))
    }
}

/// Whether the workspace-relative directory `package` holds a `BUILD.bazel`
/// or `BUILD` file. `.bazelignore` and `--deleted_packages` are not consulted.
pub fn is_package(workspace_root: &Path, package: &str) -> bool {
    let dir = workspace_root.join(package);
    ["BUILD.bazel", "BUILD"]
        .iter()
        .any(|name| dir.join(name).is_file())
}

/// Resolve a label written relative to `current_package`, the
/// workspace-relative directory a command runs from (`""` for the root), into
/// an absolute label, following Bazel's target-pattern rule
/// (<https://bazel.build/run/build#specifying-build-targets>). An absolute
/// label is returned unchanged.
///
/// - `:tool` names a target in `current_package`, `sub:tool` one in
///   `current_package/sub`.
/// - A label with no `:` names a path, `current_package/<label>`. If that
///   path is a package, the label is its implicit target (`//foo/tool:tool`).
///   Otherwise it names a target in the longest enclosing package, which may
///   lie above `current_package`: `x` from `foo/bar` is `//foo:bar/x` when
///   only `foo` is a package.
/// - When no enclosing directory is a package, Bazel rejects the label; it
///   resolves as if the path were a package.
///
/// `is_package` answers whether a workspace-relative directory is a package.
pub fn resolve_relative_label(
    label: &str,
    current_package: &str,
    is_package: impl Fn(&str) -> bool,
) -> String {
    if label.starts_with("//") || label.starts_with('@') {
        return label.to_owned();
    }
    if let Some((pkg, name)) = label.split_once(':') {
        return format!("//{}:{name}", join_package(current_package, pkg));
    }
    let path = join_package(current_package, label);
    let mut package = path.as_str();
    loop {
        if is_package(package) {
            return if package == path {
                let name = path.rsplit('/').next().unwrap_or(&path);
                format!("//{path}:{name}")
            } else {
                let rest = path[package.len()..].trim_start_matches('/');
                format!("//{package}:{rest}")
            };
        }
        if package.is_empty() {
            break;
        }
        package = package.rsplit_once('/').map_or("", |(parent, _)| parent);
    }
    let name = path.rsplit('/').next().unwrap_or(&path);
    format!("//{path}:{name}")
}

fn join_package(current: &str, rel: &str) -> String {
    match (current.is_empty(), rel.is_empty()) {
        (true, _) => rel.to_owned(),
        (false, true) => current.to_owned(),
        (false, false) => format!("{current}/{rel}"),
    }
}

#[cfg(test)]
mod tests {
    use super::{LabelResolver, normalize_label, resolve_relative_label};

    /// The cases the AXL copy's tests run too.
    #[test]
    fn shared_cases() {
        let cases: Vec<(String, String, String)> =
            serde_json::from_str(include_str!("label_cases.json")).unwrap();
        assert!(!cases.is_empty());
        for (input, expected, description) in cases {
            assert_eq!(normalize_label(&input), expected, "{description}: {input}");
        }
    }

    #[test]
    fn every_spelling_of_a_main_repository_target_agrees() {
        for spelling in ["//pkg", "//pkg:pkg", "@//pkg", "@@//pkg:pkg"] {
            assert_eq!(normalize_label(spelling), "//pkg:pkg", "{spelling}");
        }
    }

    /// The scratch layout the cases below were checked against with Bazel
    /// 9.0.1 (`bazel query <label>` from each working directory).
    const PACKAGES: &[&str] = &["", "foo", "foo/sub"];

    fn resolve(label: &str, current_package: &str) -> String {
        resolve_relative_label(label, current_package, |p| PACKAGES.contains(&p))
    }

    #[test]
    fn relative_labels_resolve_as_bazel_reads_them() {
        for (cwd, label, expected) in [
            ("", "tool", "//:tool"),
            ("", "a/deep", "//:a/deep"),
            ("a", "deep", "//:a/deep"),
            ("", "foo/sub", "//foo/sub:sub"),
            ("", "foo/bar/x", "//foo:bar/x"),
            ("foo", "tool", "//foo:tool"),
            ("foo", "sub", "//foo/sub:sub"),
            ("foo", "bar/x", "//foo:bar/x"),
            ("foo", "sub:t", "//foo/sub:t"),
            ("foo", ":tool", "//foo:tool"),
            ("foo/bar", "x", "//foo:bar/x"),
            ("foo/bar", ":x", "//foo/bar:x"),
            ("foo/sub", "t", "//foo/sub:t"),
        ] {
            assert_eq!(resolve(label, cwd), expected, "{label} from {cwd:?}");
        }
    }

    #[test]
    fn absolute_labels_are_unchanged() {
        assert_eq!(resolve("@repo//pkg", "foo"), "@repo//pkg");
        assert_eq!(resolve("//pkg", "foo"), "//pkg");
    }

    #[test]
    fn a_label_outside_every_package_resolves_as_a_package() {
        let none = |_: &str| false;
        assert_eq!(
            resolve_relative_label("tool", "foo", none),
            "//foo/tool:tool"
        );
        assert_eq!(resolve_relative_label("tool", "", none), "//tool:tool");
    }

    #[test]
    fn the_resolver_probes_build_files_in_the_workspace() {
        let ws = tempfile::tempdir().unwrap();
        let root = ws.path();
        std::fs::create_dir_all(root.join("foo/bar")).unwrap();
        std::fs::create_dir_all(root.join("foo/sub")).unwrap();
        std::fs::write(root.join("foo/BUILD.bazel"), "").unwrap();
        std::fs::write(root.join("foo/sub/BUILD"), "").unwrap();
        // A directory named BUILD does not make a package.
        std::fs::create_dir_all(root.join("foo/bar/BUILD")).unwrap();

        let labels = LabelResolver::new(root, &root.join("foo/bar"));
        assert_eq!(labels.current_package(), "foo/bar");
        assert_eq!(labels.resolve("x"), "//foo:bar/x");
        assert_eq!(
            LabelResolver::new(root, &root.join("foo")).normalize("sub"),
            "//foo/sub:sub"
        );
        assert_eq!(
            LabelResolver::new(root, root).normalize("@@//foo"),
            "//foo:foo"
        );
    }

    #[test]
    fn outside_the_workspace_labels_resolve_against_the_root() {
        let ws = tempfile::tempdir().unwrap();
        let elsewhere = tempfile::tempdir().unwrap();
        let labels = LabelResolver::new(ws.path(), elsewhere.path());
        assert_eq!(labels.current_package(), "");
        assert_eq!(labels.resolve(":tool"), "//:tool");
    }
}
