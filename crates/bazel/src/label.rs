//! Label normalization for matching what a task asked to run against what
//! BES reports.
//!
//! Bazel surfaces two label forms depending on context: the apparent form
//! users write (`@module//pkg:tgt`) and the canonical form BES emits for
//! bzlmod repositories (`@@module+ver//pkg:tgt` on Bazel 8+,
//! `@@module~//pkg:tgt` on Bazel 7). Relative labels (`:foo`, `foo`,
//! `pkg:foo`) resolve against the invoking package, and a label with no `:`
//! gets its implicit target name. [`apparent_label`] maps every form to one
//! key.

/// Normalize `label` to apparent form with an explicit target name.
///
/// `current_package` is the workspace-relative path of the package the task
/// was invoked from (`""` for the root); only relative labels consult it.
///
/// Canonical to apparent rules:
/// - module repo `@@module+version//...` or `@@module~//...` → `@module//...`
/// - extension repo `@@module+ext+repo//...` or `@@module~~ext~repo//...`
///   → `@repo//...`
pub fn apparent_label(label: &str, current_package: &str) -> String {
    let mut label = label.to_owned();
    if let Some(rest) = label.strip_prefix("@@") {
        if let Some(idx) = rest.find("//") {
            let canonical_repo = &rest[..idx];
            // `~` is Bazel 7's separator and never valid in a version, so its
            // presence is unambiguous; `+` can appear in semver build metadata.
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

    if !label.starts_with("//") && !label.starts_with('@') {
        if label.starts_with(':') {
            return format!("//{current_package}{label}");
        }
        if let Some((pkg, name)) = label.split_once(':') {
            let full = join_package(current_package, pkg);
            return format!("//{full}:{name}");
        }
        let full = join_package(current_package, &label);
        let name = full.rsplit('/').next().unwrap_or(&full).to_owned();
        return format!("//{full}:{name}");
    }

    if label.contains(':') {
        return label;
    }
    let (repo, path) = match label.find("//") {
        Some(idx) => (&label[..idx], &label[idx + 2..]),
        None => return label,
    };
    let name = path.rsplit('/').next().unwrap_or(path);
    if path.is_empty() {
        return label;
    }
    format!("{repo}//{path}:{name}")
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
    use super::apparent_label;

    #[test]
    fn absolute_labels_get_an_implicit_name() {
        assert_eq!(apparent_label("//foo/bar", ""), "//foo/bar:bar");
        assert_eq!(apparent_label("//foo/bar:baz", ""), "//foo/bar:baz");
        assert_eq!(apparent_label("@repo//pkg", ""), "@repo//pkg:pkg");
    }

    #[test]
    fn canonical_labels_become_apparent() {
        assert_eq!(
            apparent_label("@@rules_go+//go:tool", ""),
            "@rules_go//go:tool"
        );
        assert_eq!(
            apparent_label("@@rules_go+0.50.1//go:tool", ""),
            "@rules_go//go:tool"
        );
        assert_eq!(
            apparent_label("@@rules_go~//go:tool", ""),
            "@rules_go//go:tool"
        );
        assert_eq!(apparent_label("@@mod+ext+repo//pkg:t", ""), "@repo//pkg:t");
        assert_eq!(apparent_label("@@mod~~ext~repo//pkg:t", ""), "@repo//pkg:t");
        assert_eq!(apparent_label("@@foo~1.2.3+meta//pkg:t", ""), "@foo//pkg:t");
        assert_eq!(apparent_label("@@//pkg:t", ""), "@//pkg:t");
    }

    #[test]
    fn relative_labels_resolve_against_the_package() {
        assert_eq!(apparent_label(":tool", "foo/bar"), "//foo/bar:tool");
        assert_eq!(apparent_label("tool", "foo/bar"), "//foo/bar/tool:tool");
        assert_eq!(apparent_label("sub:tool", "foo"), "//foo/sub:tool");
        assert_eq!(apparent_label(":tool", ""), "//:tool");
        assert_eq!(apparent_label("tool", ""), "//tool:tool");
    }
}
