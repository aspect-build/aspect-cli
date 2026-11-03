//! Label normalization for matching what a task asked to run against what
//! BES reports.
//!
//! [`normalize_label`] maps every spelling of a target to one key. It mirrors
//! `normalize_label` in `crates/aspect-cli/src/builtins/aspect/private/lib/runnable.axl`,
//! which the AXL tasks use for the same matching; the two must agree on every
//! label; both test suites run the cases in `label_cases.json`.
//! [`resolve_relative_label`] turns a label written relative to a working
//! directory into an absolute one first, and [`normalize_label_from`] does
//! both.

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
/// are packages, which this function cannot see; resolve it first, as
/// [`resolve_relative_label`] does.
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

/// Resolve a label written relative to `current_package`, the
/// workspace-relative directory the task was invoked from (`""` for the
/// root), into an absolute label; an absolute label is returned unchanged.
/// `:tool` and `sub:tool` name a target in `current_package` and in
/// `current_package/sub`. A bare name is read as a package, so `tool` is
/// `//<current_package>/tool:tool`. That is Bazel's reading when
/// `current_package/tool` is a package; when it is not, Bazel picks the
/// nearest enclosing package instead, which this function cannot see.
pub fn resolve_relative_label(label: &str, current_package: &str) -> String {
    if label.starts_with("//") || label.starts_with('@') {
        return label.to_owned();
    }
    if label.starts_with(':') {
        return format!("//{current_package}{label}");
    }
    if let Some((pkg, name)) = label.split_once(':') {
        return format!("//{}:{name}", join_package(current_package, pkg));
    }
    let full = join_package(current_package, label);
    let name = full.rsplit('/').next().unwrap_or(&full).to_owned();
    format!("//{full}:{name}")
}

/// [`normalize_label`] of a label written relative to `current_package`; see
/// [`resolve_relative_label`].
pub fn normalize_label_from(label: &str, current_package: &str) -> String {
    normalize_label(&resolve_relative_label(label, current_package))
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
    use super::{normalize_label, normalize_label_from, resolve_relative_label};

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

    #[test]
    fn relative_labels_resolve_against_the_working_package() {
        assert_eq!(resolve_relative_label(":tool", "foo/bar"), "//foo/bar:tool");
        assert_eq!(
            resolve_relative_label("tool", "foo/bar"),
            "//foo/bar/tool:tool"
        );
        assert_eq!(resolve_relative_label("sub:tool", "foo"), "//foo/sub:tool");
        assert_eq!(resolve_relative_label(":tool", ""), "//:tool");
        assert_eq!(resolve_relative_label("tool", ""), "//tool:tool");
        assert_eq!(resolve_relative_label("@repo//pkg", "foo"), "@repo//pkg");
        assert_eq!(resolve_relative_label("//pkg", "foo"), "//pkg");
    }

    #[test]
    fn normalize_label_from_resolves_then_normalizes() {
        assert_eq!(normalize_label_from(":tool", "foo"), "//foo:tool");
        assert_eq!(normalize_label_from("tool", "foo"), "//foo/tool:tool");
        assert_eq!(normalize_label_from("@@//pkg", "foo"), "//pkg:pkg");
        assert_eq!(
            normalize_label_from("@@rules_go+//go", "foo"),
            "@rules_go//go:go"
        );
    }
}
