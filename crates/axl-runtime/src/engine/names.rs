/// Name validation and conversion utilities for task, feature, trait, arg, and command names.

/// Convert a snake_case or CamelCase identifier to a kebab-case CLI command name.
///
/// | Variable name    | CLI command      |
/// |------------------|------------------|
/// | `axl_add`        | `axl-add`        |
/// | `AxlAdd`         | `axl-add`        |
/// | `ci_build`       | `ci-build`       |
/// | `CIBuild`        | `ci-build`       |
/// | `s3_upload`      | `s3-upload`      |
/// | `S3Upload`       | `s3-upload`      |
/// | `https_redirect` | `https-redirect` |
/// | `HTTPSRedirect`  | `https-redirect` |
pub fn to_command_name(var_name: &str) -> String {
    var_name
        .split('_')
        .flat_map(camel_to_kebab_words)
        .filter(|w| !w.is_empty())
        .collect::<Vec<_>>()
        .join("-")
}

/// Convert a snake_case or kebab-case identifier to a Title Case display name.
/// `"artifact_upload"` → `"Artifact Upload"`, `"github-status-checks"` → `"Github Status Checks"`.
pub fn to_display_name(name: &str) -> String {
    name.split(|c| c == '_' || c == '-')
        .filter(|seg| !seg.is_empty())
        .map(|seg| {
            let mut chars = seg.chars();
            match chars.next() {
                None => String::new(),
                Some(first) => {
                    let upper: String = first.to_uppercase().collect();
                    upper + chars.as_str()
                }
            }
        })
        .collect::<Vec<_>>()
        .join(" ")
}

/// Convert a CamelCase identifier to a Title Case display name with spaces.
/// `"ArtifactUpload"` → `"Artifact Upload"`, `"CIBuild"` → `"Ci Build"`.
pub fn camel_to_display_name(camel: &str) -> String {
    camel_to_kebab_words(camel)
        .into_iter()
        .map(|w| {
            let mut chars = w.chars();
            match chars.next() {
                None => String::new(),
                Some(first) => {
                    let upper: String = first.to_uppercase().collect();
                    upper + chars.as_str()
                }
            }
        })
        .collect::<Vec<_>>()
        .join(" ")
}

pub(crate) fn camel_to_kebab_words(s: &str) -> Vec<String> {
    if s.is_empty() {
        return vec![];
    }
    let chars: Vec<char> = s.chars().collect();
    let mut words: Vec<String> = Vec::new();
    let mut current = String::new();

    for (i, &c) in chars.iter().enumerate() {
        if c.is_ascii_uppercase() {
            let prev = if i > 0 { Some(chars[i - 1]) } else { None };
            let next = chars.get(i + 1).copied();
            let split = match prev {
                None => false,
                Some(p) => {
                    p.is_ascii_lowercase()
                        || p.is_ascii_digit()
                        || (p.is_ascii_uppercase()
                            && next.map_or(false, |n| n.is_ascii_lowercase()))
                }
            };
            if split && !current.is_empty() {
                words.push(current.to_lowercase());
                current = String::new();
            }
            current.push(c);
        } else {
            current.push(c);
        }
    }
    if !current.is_empty() {
        words.push(current.to_lowercase());
    }
    words
}

/// Validate that a feature or trait export name conforms to `[A-Z][A-Za-z0-9]*` (CamelCase).
///
/// Features and traits are referenced as map keys (`ctx.features[ArtifactUpload]`,
/// `ctx.traits[MyConfig]`), which reads like a type key. CamelCase is enforced to
/// match Bazel's provider convention (`dep[CcInfo]`) and signal this type-key role.
pub fn validate_type_name(name: &str, kind: &str) -> Result<(), String> {
    let mut chars = name.chars();
    match chars.next() {
        None => return Err(format!("{kind} name cannot be empty")),
        Some(c) if !c.is_ascii_uppercase() => {
            return Err(format!(
                "{kind} name {:?} must start with an uppercase letter (CamelCase required)",
                name
            ));
        }
        _ => {}
    }
    for c in chars {
        if !c.is_ascii_alphanumeric() {
            return Err(format!(
                "{kind} name {:?} contains invalid character {:?} (allowed: A-Z, a-z, 0-9)",
                name, c
            ));
        }
    }
    Ok(())
}

/// Validate that an arg name conforms to `[a-z][a-z0-9_]*`.
pub fn validate_arg_name(name: &str) -> Result<(), String> {
    let mut chars = name.chars();
    match chars.next() {
        None => return Err("arg name cannot be empty".to_string()),
        Some(c) if !c.is_ascii_lowercase() => {
            return Err(format!(
                "arg name {:?} must start with a lowercase letter",
                name
            ));
        }
        _ => {}
    }
    for c in chars {
        if !c.is_ascii_lowercase() && !c.is_ascii_digit() && c != '_' {
            return Err(format!(
                "arg name {:?} contains invalid character {:?} (allowed: a-z, 0-9, _)",
                name, c
            ));
        }
    }
    Ok(())
}

/// Validate that a command/group name conforms to `[a-z][a-z0-9-]*` with no
/// trailing dash and no consecutive dashes.
pub fn validate_command_name(name: &str, kind: &str) -> Result<(), String> {
    let mut chars = name.chars();
    match chars.next() {
        None => return Err(format!("{kind} name cannot be empty")),
        Some(c) if !c.is_ascii_lowercase() => {
            return Err(format!(
                "{kind} name {:?} must start with a lowercase letter",
                name
            ));
        }
        _ => {}
    }
    for c in chars {
        if !c.is_ascii_lowercase() && !c.is_ascii_digit() && c != '-' {
            return Err(format!(
                "{kind} name {:?} contains invalid character {:?} (allowed: a-z, 0-9, -)",
                name, c
            ));
        }
    }
    if name.ends_with('-') {
        return Err(format!("{kind} name {:?} cannot end with a dash", name));
    }
    if name.contains("--") {
        return Err(format!(
            "{kind} name {:?} cannot contain consecutive dashes",
            name
        ));
    }
    Ok(())
}
