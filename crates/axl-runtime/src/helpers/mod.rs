use std::path::{Component, Path, PathBuf};

use anyhow::anyhow;

// Constants for special directory names used in module resolution.
// These define the structure for local modules (e.g., .aspect/axl/module_name).
pub const ASPECT_ROOT: &str = ".aspect";
pub const AXL_MODULE_DIR: &str = "modules";

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
pub fn validate_module_name(module_name: &str) -> starlark::Result<()> {
    if module_name.is_empty() {
        return Err(starlark::Error::new_other(anyhow!(
            "Module name cannot be empty"
        )));
    }

    // Begins with lowercase letter
    let first_char = module_name.chars().next().unwrap();
    if !first_char.is_ascii_lowercase() {
        return Err(starlark::Error::new_other(anyhow!(
            "Module name must begin with a lowercase letter"
        )));
    }

    // Ends with lowercase letter or digit
    let last_char = module_name.chars().last().unwrap();
    if !last_char.is_ascii_lowercase() && !last_char.is_ascii_digit() {
        return Err(starlark::Error::new_other(anyhow!(
            "Module name must end with a lowercase letter or digit"
        )));
    }

    // Only allowed characters
    let allowed = "abcdefghijklmnopqrstuvwxyz0123456789.-_";
    for c in module_name.chars() {
        if !allowed.contains(c) {
            return Err(starlark::Error::new_other(anyhow!(
                "Module name contains invalid character: '{}'",
                c
            )));
        }
    }

    Ok(())
}

/// Sanitizes a load path string according to various rules and extracts components.
/// Rules include:
/// - No leading or trailing whitespace.
/// - Does not start with '/'.
/// - No '\' separators.
/// - No double '//' separators.
/// - Does not start with '@@'.
/// - Must contain at least one '/' and end with '/filename.axl'.
/// - If starts with '@', validates the module name after '@' and before first '/'.
/// - Validates each path segment after the module (if present) as a valid filename.
/// - Allows starting with a single './' and zero or more '../' in the initial relative prefix, but no multiple '.' segments and no '.' or '..' after normal segments.
///
/// # Arguments
/// * `load_path` - The load path string to validate.
///
/// # Returns
/// * `Ok((Option<String>, PathBuf))` where the first element is the module name if present,
///   and the second is the normalized path with the module prefix stripped (if it was present).
/// * `Err(starlark::Error)` with a descriptive error if any validation fails.
pub fn sanitize_load_path_lexically(
    load_path: &str,
) -> starlark::Result<(Option<String>, PathBuf)> {
    if load_path.trim() != load_path {
        return Err(starlark::Error::new_other(anyhow!(
            "Paths starting or ending with whitespace are disallowed"
        )));
    }

    if load_path.starts_with('/') {
        return Err(starlark::Error::new_other(anyhow!(
            "Paths starting with '/' are disallowed"
        )));
    }

    if load_path.contains('\\') {
        return Err(starlark::Error::new_other(anyhow!(
            "Paths containing '\\' separators are disallowed"
        )));
    }

    if load_path.contains("//") {
        return Err(starlark::Error::new_other(anyhow!(
            "Load paths with double slashes '//' are disallowed"
        )));
    }

    if load_path.starts_with("@@") {
        return Err(starlark::Error::new_other(anyhow!(
            "Paths starting with '@@' are disallowed"
        )));
    }

    // Check that the path ends with /filename.axl
    let last_slash_pos = match load_path.rfind('/') {
        Some(pos) => pos,
        None => {
            return Err(starlark::Error::new_other(anyhow!(
                "Load path must contain at least one '/' separator and end with a filename ending in '.axl'"
            )));
        }
    };
    let filename = &load_path[(last_slash_pos + 1)..];
    if filename.is_empty() || !filename.ends_with(".axl") {
        return Err(starlark::Error::new_other(anyhow!(
            "Load path must end with a filename ending in '.axl'"
        )));
    }

    let (module_name_option, path_to_validate): (Option<String>, &str) =
        if load_path.starts_with('@') {
            // Must have at least one / (already checked via rfind above)

            // Extract module_name: between @ and first /
            let first_slash_pos = load_path.find('/').unwrap(); // Safe due to prior check
            let module_name = &load_path[1..first_slash_pos];

            validate_module_name(module_name)?;

            (
                Some(module_name.to_string()),
                &load_path[(first_slash_pos + 1)..],
            )
        } else {
            (None, load_path)
        };

    // Validate path segments after module (if present)
    let mut allowing_relative = true;
    let mut seen_dot = false;
    let mut aspect_root_count = 0;
    for segment in path_to_validate.split('/') {
        if segment == ASPECT_ROOT {
            aspect_root_count += 1;
            if aspect_root_count > 1 {
                return Err(starlark::Error::new_other(anyhow!(
                    "Load paths with multiple '{ASPECT_ROOT}' segments are disallowed"
                )));
            }
        }
        if allowing_relative {
            if segment == "." {
                if seen_dot {
                    return Err(starlark::Error::new_other(anyhow!(
                        "Multiple '.' relative segments are disallowed"
                    )));
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

    Ok((
        module_name_option,
        normalize_load_path_lexically(Path::new(path_to_validate)),
    ))
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
pub fn normalize_abs_path_lexically(path: &Path) -> starlark::Result<PathBuf> {
    if !path.is_absolute() {
        return Err(starlark::Error::new_other(anyhow!(
            "Path is not absolute: {}",
            path.display()
        )));
    }

    let mut iter = path.components();

    if iter.next() != Some(Component::RootDir) {
        return Err(starlark::Error::new_other(anyhow!(
            "Path does not start with root directory"
        )));
    }

    let next = iter.next();
    if matches!(next, Some(Component::CurDir) | Some(Component::ParentDir)) {
        return Err(starlark::Error::new_other(anyhow!(
            "Absolute path starts with invalid segment '.' or '..'"
        )));
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
fn normalize_load_path_lexically(path: &Path) -> PathBuf {
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
fn validate_path_segment(segment: &str) -> starlark::Result<()> {
    if segment.is_empty() {
        return Err(starlark::Error::new_other(anyhow!(
            "Empty path segment disallowed"
        )));
    }

    if segment.trim() != segment {
        return Err(starlark::Error::new_other(anyhow!(
            "Path segment '{}' contains leading or trailing whitespace, which is disallowed for cross-platform safety",
            segment
        )));
    }

    if segment.as_bytes().len() > 255 {
        return Err(starlark::Error::new_other(anyhow!(
            "Path segment '{}' exceeds 255 bytes",
            segment
        )));
    }

    if segment.ends_with(' ') || segment.ends_with('.') {
        return Err(starlark::Error::new_other(anyhow!(
            "Path segment '{}' ends with disallowed character (space or dot)",
            segment
        )));
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
        return Err(starlark::Error::new_other(anyhow!(
            "Reserved name '{}' disallowed in path segment",
            segment
        )));
    }

    for c in segment.chars() {
        if c.is_control() || matches!(c, '\\' | '/' | ':' | '*' | '?' | '"' | '<' | '>' | '|') {
            return Err(starlark::Error::new_other(anyhow!(
                "Invalid character '{}' in path segment '{}'",
                c,
                segment
            )));
        }
    }

    Ok(())
}
