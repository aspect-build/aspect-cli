use std::collections::HashMap;
use std::path::{Path, PathBuf};

use crate::{BazelRcError, RcOption, preprocess, tokenize};

/// Recursively parse a bazelrc file, resolving imports.
///
/// `version_condition` carries the condition string from an enclosing
/// `try-import-if-bazel-version` directive. It is `None` for unconditional files.
/// `import_stack` tracks the current recursion chain for cycle detection.
/// Diamond imports (A→B, A→C, B→D, C→D) are allowed — only true cycles error.
pub(crate) fn parse_file(
    path: &Path,
    workspace_root: &Path,
    version_condition: Option<String>,
    sources: &mut Vec<PathBuf>,
    options: &mut HashMap<String, Vec<RcOption>>,
    import_stack: &mut Vec<PathBuf>,
) -> Result<(), BazelRcError> {
    let canonical = path.canonicalize().map_err(|e| BazelRcError::Io {
        file: path.to_path_buf(),
        source: e,
    })?;

    // Cycle detection: only the current recursion stack
    if import_stack.contains(&canonical) {
        let chain: Vec<String> = import_stack
            .iter()
            .map(|p| p.display().to_string())
            .chain(std::iter::once(canonical.display().to_string()))
            .collect();
        return Err(BazelRcError::ImportCycle { chain });
    }

    import_stack.push(canonical.clone());
    let source_index = sources.len();
    sources.push(canonical.clone());

    let content = std::fs::read_to_string(&canonical).map_err(|e| BazelRcError::Io {
        file: canonical.clone(),
        source: e,
    })?;

    let lines = preprocess::process(&content);

    for line in lines {
        let tokens = tokenize::tokenize(&line);
        if tokens.is_empty() {
            continue;
        }

        match tokens[0].as_str() {
            "import" => {
                if tokens.len() != 2 {
                    return Err(BazelRcError::InvalidImportArgs {
                        directive: line.clone(),
                    });
                }
                let import_path = resolve_import_path(&tokens[1], workspace_root);
                if !import_path.exists() {
                    return Err(BazelRcError::ImportNotFound { path: import_path });
                }
                parse_file(
                    &import_path,
                    workspace_root,
                    version_condition.clone(),
                    sources,
                    options,
                    import_stack,
                )?;
            }
            "try-import" => {
                if tokens.len() != 2 {
                    return Err(BazelRcError::InvalidImportArgs {
                        directive: line.clone(),
                    });
                }
                let import_path = resolve_import_path(&tokens[1], workspace_root);
                if import_path.exists() {
                    parse_file(
                        &import_path,
                        workspace_root,
                        version_condition.clone(),
                        sources,
                        options,
                        import_stack,
                    )?;
                }
            }
            "try-import-if-bazel-version" => {
                if tokens.len() != 3 {
                    return Err(BazelRcError::InvalidImportArgs {
                        directive: line.clone(),
                    });
                }
                let condition = tokens[1].clone();
                let import_path = resolve_import_path(&tokens[2], workspace_root);
                // Always try-import (skip only if file missing); tag flags with condition.
                // Version evaluation is deferred to the caller.
                if import_path.exists() {
                    parse_file(
                        &import_path,
                        workspace_root,
                        Some(condition),
                        sources,
                        options,
                        import_stack,
                    )?;
                }
            }
            directive if directive.contains(':') || !directive.starts_with('-') => {
                // command key is tokens[0]; each subsequent token is a separate RcOption
                let key = tokens[0].clone();
                let entry = options.entry(key.clone()).or_default();
                for value in &tokens[1..] {
                    entry.push(RcOption {
                        value: value.clone(),
                        command: key.clone(),
                        source_index,
                        version_condition: version_condition.clone(),
                    });
                }
            }
            _ => {
                // Lines starting with '-' that aren't import directives
                let key = tokens[0].clone();
                let entry = options.entry(key.clone()).or_default();
                for value in &tokens[1..] {
                    entry.push(RcOption {
                        value: value.clone(),
                        command: key.clone(),
                        source_index,
                        version_condition: version_condition.clone(),
                    });
                }
            }
        }
    }

    import_stack.pop();
    Ok(())
}

/// Resolve import path: `%workspace%/rest` → `workspace_root/rest`, otherwise as-is.
fn resolve_import_path(raw: &str, workspace_root: &Path) -> PathBuf {
    if let Some(rest) = raw.strip_prefix("%workspace%/") {
        workspace_root.join(rest)
    } else {
        PathBuf::from(raw)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    fn parse(path: &Path, root: &Path) -> Result<HashMap<String, Vec<RcOption>>, BazelRcError> {
        let mut sources = Vec::new();
        let mut options = HashMap::new();
        let mut stack = Vec::new();
        parse_file(path, root, None, &mut sources, &mut options, &mut stack)?;
        Ok(options)
    }

    // ── Import argument validation ────────────────────────────────────────────

    #[test]
    fn import_too_many_args() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        let rc = root.join("test.bazelrc");
        fs::write(&rc, "import foo bar\n").unwrap();
        let err = parse(&rc, root).unwrap_err();
        assert!(matches!(err, BazelRcError::InvalidImportArgs { .. }));
    }

    #[test]
    fn try_import_too_many_args() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        let rc = root.join("test.bazelrc");
        fs::write(&rc, "try-import foo bar\n").unwrap();
        let err = parse(&rc, root).unwrap_err();
        assert!(matches!(err, BazelRcError::InvalidImportArgs { .. }));
    }

    #[test]
    fn try_import_if_bazel_version_too_few_args() {
        // Only 2 tokens: directive + version condition, missing path
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        let rc = root.join("test.bazelrc");
        fs::write(&rc, "try-import-if-bazel-version >=8\n").unwrap();
        let err = parse(&rc, root).unwrap_err();
        assert!(matches!(err, BazelRcError::InvalidImportArgs { .. }));
    }

    // ── %workspace% path resolution ───────────────────────────────────────────

    #[test]
    fn workspace_relative_import_resolves() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();

        let sub = root.join("sub.bazelrc");
        fs::write(&sub, "build --sub-flag\n").unwrap();

        let main_rc = root.join("main.bazelrc");
        fs::write(&main_rc, "import %workspace%/sub.bazelrc\n").unwrap();

        let opts = parse(&main_rc, root).unwrap();
        let build = opts.get("build").expect("build key");
        assert_eq!(build.len(), 1);
        assert_eq!(build[0].value, "--sub-flag");
    }

    // ── Import ordering ───────────────────────────────────────────────────────

    #[test]
    fn import_maintains_ordering() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();

        let imported = root.join("imported.bazelrc");
        fs::write(&imported, "build --from-import\n").unwrap();

        let main_rc = root.join("main.bazelrc");
        fs::write(
            &main_rc,
            format!(
                "build --before\nimport {}\nbuild --after\n",
                imported.display()
            ),
        )
        .unwrap();

        let opts = parse(&main_rc, root).unwrap();
        let values: Vec<&str> = opts["build"].iter().map(|o| o.value.as_str()).collect();
        assert_eq!(values, vec!["--before", "--from-import", "--after"]);
    }

    // ── Long import chain cycle detection ─────────────────────────────────────

    #[test]
    fn long_import_chain_cycle() {
        // A → B → C → D → B  (B appears twice → cycle)
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();

        let b = root.join("b.bazelrc");
        let c = root.join("c.bazelrc");
        let d = root.join("d.bazelrc");
        let a = root.join("a.bazelrc");

        fs::write(&d, format!("import {}\n", b.display())).unwrap();
        fs::write(&c, format!("import {}\n", d.display())).unwrap();
        fs::write(&b, format!("import {}\n", c.display())).unwrap();
        fs::write(&a, format!("import {}\n", b.display())).unwrap();

        let err = parse(&a, root).unwrap_err();
        assert!(matches!(err, BazelRcError::ImportCycle { .. }));
    }
}
