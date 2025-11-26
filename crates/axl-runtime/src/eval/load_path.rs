use std::path::{Component, Path, PathBuf};

use anyhow::anyhow;

/// Joins two paths ensuring that the subpath does not lead to oustide of base.
pub(super) fn join_confined(base: &Path, subpath: &Path) -> anyhow::Result<PathBuf> {
    let mut dest = base.to_path_buf();
    for component in subpath.components() {
        match component {
            Component::CurDir => {}
            Component::ParentDir => {
                if dest == base {
                    return Err(anyhow!("subpath {:?} is outside of {:?}", subpath, base));
                }
                dest.pop();
            }
            Component::Normal(s) => {
                dest.push(s);
            }
            comp => {
                return Err(anyhow!(
                    "invalid component `{:?}` in path: {:?}",
                    comp,
                    subpath
                ));
            }
        }
    }
    Ok(dest)
}

/// Validates a module name according to the following rules:
/// - Must not be empty.
/// - Must begin with a lowercase letter (a-z).
/// - Must end with a lowercase letter (a-z) or digit (0-9).
/// - Can only contain lowercase letters (a-z), digits (0-9), dots (.), hyphens (-), and underscores (_).
///
/// # Arguments
/// * `module_name` - The module name string to validate.
///
/// # Returns
/// * `Ok(())` if the module name is valid.
/// * `Err(starlark::Error)` with a descriptive error if invalid.
pub fn validate_module_name(module_name: &str) -> anyhow::Result<()> {
    if module_name.is_empty() {
        return Err(anyhow!("module name cannot be empty"));
    }

    // Begins with lowercase letter
    let first_char = module_name.chars().next().unwrap();
    if !first_char.is_ascii_lowercase() {
        return Err(anyhow!("module name must begin with a lowercase letter"));
    }

    // Ends with lowercase letter or digit
    let last_char = module_name.chars().last().unwrap();
    if !last_char.is_ascii_lowercase() && !last_char.is_ascii_digit() {
        return Err(anyhow!(
            "module name must end with a lowercase letter or digit"
        ));
    }

    // Only allowed characters
    let allowed = "abcdefghijklmnopqrstuvwxyz0123456789.-_";
    for c in module_name.chars() {
        if !allowed.contains(c) {
            return Err(anyhow!("module name contains invalid character: '{}'", c));
        }
    }

    Ok(())
}

/// Normalizes an absolute path by removing redundant '.' components and resolving '..' components against preceding normal components where possible.
/// Ensures the path is absolute and that the first segment after the root is not '.' or '..'.
/// Ignores extra '..' components that would go beyond the root without preserving them.
/// This normalization is purely syntactic and does not interact with the filesystem.
/// TODO: switch to Path.normalize_lexically in the future once it is in a stable Rust release: https://github.com/rust-lang/rust/issues/134694.
///
/// # Arguments
/// * `path` - The absolute path to normalize.
///
/// # Returns
/// * `Ok(PathBuf)` containing the normalized path if valid.
/// * `Err(starlark::Error)` if the path is not absolute or starts with an invalid segment after the root.
pub fn normalize_abs_path_lexically(path: &Path) -> anyhow::Result<PathBuf> {
    if !path.is_absolute() {
        return Err(anyhow!("path is not absolute: {}", path.display()));
    }

    let mut iter = path.components();

    if iter.next() != Some(Component::RootDir) {
        return Err(anyhow!("path does not start with root directory"));
    }

    let next = iter.next();
    if matches!(next, Some(Component::CurDir) | Some(Component::ParentDir)) {
        return Err(anyhow!(
            "absolute path starts with invalid segment '.' or '..'"
        ));
    }

    let mut components = vec![Component::RootDir];

    if let Some(c) = next {
        components.push(c);
    }

    for component in iter {
        match component {
            Component::ParentDir => {
                if !components.is_empty() && matches!(components.last(), Some(Component::Normal(_)))
                {
                    components.pop();
                }
                // Ignore if at root; do not push
            }
            Component::CurDir => {}
            _ => components.push(component),
        }
    }

    let mut result = PathBuf::new();
    for c in components {
        result.push(c.as_os_str());
    }

    Ok(result)
}

/// Normalizes a relative path by removing redundant '.' components and resolving '..' components against preceding normal components where possible.
/// Unresolvable '..' components (e.g., at the beginning or following other '..') are preserved to maintain the relative nature of the path.
/// If the original path starts with './' and the normalized path does not begin with '.' or '..', the leading './' is preserved for explicit current directory reference.
/// This normalization is purely syntactic and does not interact with the filesystem.
fn normalize_rel_path_lexically(path: &Path) -> PathBuf {
    let starts_with_cur = path.components().next() == Some(Component::CurDir);

    let mut components = Vec::new();
    for component in path.components() {
        match component {
            Component::ParentDir => {
                if !components.is_empty() && matches!(components.last(), Some(Component::Normal(_)))
                {
                    components.pop();
                } else {
                    components.push(component);
                }
            }
            Component::CurDir => {}
            _ => components.push(component),
        }
    }

    let mut result = PathBuf::new();
    for c in components {
        result.push(c.as_os_str());
    }

    let first_comp = result.components().next();
    if starts_with_cur
        && first_comp != Some(Component::ParentDir)
        && first_comp != Some(Component::CurDir)
    {
        PathBuf::from(".").join(&result)
    } else {
        result
    }
}

/// Validates a path segment to ensure it is a valid filename compatible with Linux, macOS, and Windows.
/// Checks include:
/// - Not empty.
/// - Length <= 255 bytes.
/// - Does not end with space or dot.
/// - Not a reserved Windows device name (e.g., CON, PRN, etc., case-insensitive, including extensions like CON.txt).
/// - No control characters or invalid characters: \, /, :, *, ?, ", <, >, |.
/// - No leading or trailing whitespace for cross-platform safety.
///
/// # Arguments
/// * `segment` - The path segment string to validate.
///
/// # Returns
/// * `Ok(())` if the segment is valid.
/// * `Err(starlark::Error)` with a descriptive error if invalid.
fn validate_path_segment(segment: &str) -> anyhow::Result<()> {
    if segment.is_empty() {
        return Err(anyhow!("empty path segment disallowed"));
    }

    if segment.trim() != segment {
        return Err(anyhow!(
            "path segment '{}' contains leading or trailing whitespace, which is disallowed for cross-platform safety",
            segment
        ));
    }

    if segment.as_bytes().len() > 255 {
        return Err(anyhow!("path segment '{}' exceeds 255 bytes", segment));
    }

    if segment.ends_with(' ') || segment.ends_with('.') {
        return Err(anyhow!(
            "path segment '{}' ends with disallowed character (space or dot)",
            segment
        ));
    }

    let upper = segment.to_uppercase();
    let base = upper.splitn(2, '.').next().unwrap();

    // These are reserved device names in Windows (case-insensitive) that cannot be used as filenames.
    // They refer to system devices: CON (console), PRN (printer), AUX (auxiliary), NUL (null),
    // COM0-COM9 (serial ports), LPT0-LPT9 (parallel ports).
    // We check the base name (before the first dot) to catch names like "CON.txt".
    let reserved = [
        "CON", "PRN", "AUX", "NUL", "COM0", "COM1", "COM2", "COM3", "COM4", "COM5", "COM6", "COM7",
        "COM8", "COM9", "LPT0", "LPT1", "LPT2", "LPT3", "LPT4", "LPT5", "LPT6", "LPT7", "LPT8",
        "LPT9",
    ];
    if reserved.contains(&base) {
        return Err(anyhow!(
            "reserved name '{}' disallowed in path segment",
            segment
        ));
    }

    for c in segment.chars() {
        if c.is_control() || matches!(c, '\\' | '/' | ':' | '*' | '?' | '"' | '<' | '>' | '|') {
            return Err(anyhow!(
                "invalid character '{}' in path segment '{}'",
                c,
                segment
            ));
        }
    }

    Ok(())
}

/// Enum representing a sanitized load path, which can be:
/// - A full module specifier like "@module_name//path/to/file.axl".
/// - A module subpath like "path/to/file.axl".
/// - A relative path like "./path/to/file.axl" or "../path/to/file.axl".
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LoadPath {
    ModuleSpecifier { module: String, subpath: PathBuf },
    ModuleSubpath(PathBuf),
    RelativePath(PathBuf),
}

impl TryFrom<&str> for LoadPath {
    type Error = anyhow::Error;

    /// Sanitizes a load path string according to various rules and extracts components.
    /// Rules include:
    /// - No leading or trailing whitespace.
    /// - Does not start with '/'.
    /// - No '\' separators.
    /// - No double '//' separators except as the module separator in full module specifiers.
    /// - Does not start with '@@'.
    /// - Must end with a filename ending in '.axl'.
    /// - If a full module specifier (starts with '@module_name//'), validates the module name after '@' and before '//'.
    /// - Validates each path segment in the subpath as a valid filename.
    /// - Allows starting with a single './' and zero or more '../' in the initial relative prefix, but no multiple '.' segments and no '.' or '..' after normal segments.
    ///
    /// # Arguments
    /// * `load_path` - The load path string to validate.
    ///
    /// # Returns
    /// * `Ok(LoadPath)` containing the parsed and normalized load path.
    /// * `Err(starlark::Error)` with a descriptive error if any validation fails.
    fn try_from(load_path: &str) -> Result<Self, Self::Error> {
        if load_path.trim() != load_path {
            return Err(anyhow!(
                "paths starting or ending with whitespace are disallowed"
            ));
        }

        if load_path.starts_with('/') {
            return Err(anyhow!("paths starting with '/' are disallowed"));
        }

        if load_path.contains('\\') {
            return Err(anyhow!("paths containing '\\' separators are disallowed"));
        }

        if load_path.starts_with("@@") {
            return Err(anyhow!("paths starting with '@@' are disallowed"));
        }

        // Extract the filename as the part after the last '/', or the whole path if no '/'
        let filename = match load_path.rfind('/') {
            Some(pos) => &load_path[(pos + 1)..],
            None => &load_path,
        };
        if filename.is_empty() || !filename.ends_with(".axl") {
            return Err(anyhow!(
                "load path must end with a filename ending in '.axl'"
            ));
        }

        let (module_name_option, path_to_validate): (Option<String>, &str) =
            if load_path.starts_with('@') {
                if let Some(double_pos) = load_path.find("//") {
                    let candidate_module = &load_path[1..double_pos];
                    if candidate_module.find('/').is_none() {
                        validate_module_name(candidate_module)?;

                        (
                            Some(candidate_module.to_string()),
                            &load_path[(double_pos + 2)..],
                        )
                    } else {
                        (None, load_path)
                    }
                } else {
                    (None, load_path)
                }
            } else {
                (None, load_path)
            };

        // Check for disallowed double slashes
        if module_name_option.is_some() {
            if path_to_validate.contains("//") {
                return Err(anyhow!(
                    "load paths with double slashes '//' are disallowed except for the module separator"
                ));
            }
        } else {
            if load_path.contains("//") {
                return Err(anyhow!(
                    "load paths with double slashes '//' are disallowed"
                ));
            }
        }

        // Validate path segments after module (if present)
        let mut allowing_relative = true;
        let mut seen_dot = false;
        for segment in path_to_validate.split('/') {
            if allowing_relative {
                if segment == "." {
                    if seen_dot {
                        return Err(anyhow!("multiple '.' relative segments are disallowed"));
                    }
                    seen_dot = true;
                    continue;
                } else if segment == ".." {
                    continue;
                } else {
                    allowing_relative = false;
                    // fall through to validate
                }
            }
            validate_path_segment(segment)?;
        }

        let normalized_path = normalize_rel_path_lexically(Path::new(path_to_validate));

        if let Some(module) = module_name_option {
            Ok(LoadPath::ModuleSpecifier {
                module,
                subpath: normalized_path,
            })
        } else {
            let first_comp = normalized_path.components().next();
            if matches!(first_comp, Some(Component::CurDir | Component::ParentDir)) {
                Ok(LoadPath::RelativePath(normalized_path))
            } else {
                Ok(LoadPath::ModuleSubpath(normalized_path))
            }
        }
    }
}
