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
