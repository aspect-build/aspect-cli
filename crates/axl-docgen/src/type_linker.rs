use crate::type_registry::TypeRegistry;
use regex::Regex;
use starlark::typing::Ty;
use std::sync::LazyLock;

/// Converts type representations to linkable strings using the `@link@` format
/// that highlight.rs expects.
pub struct TypeLinker<'a> {
    registry: &'a TypeRegistry,
    /// URL prefix (e.g. `/docs`, or `""` for absolute paths). No trailing slash.
    base_url: &'a str,
}

// Match qualified names like "module.Type" or simple identifiers
static QUALIFIED_TYPE_REGEX: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"\b([A-Za-z_][A-Za-z0-9_]*(?:\.[A-Za-z_][A-Za-z0-9_]*)*)").unwrap()
});

impl<'a> TypeLinker<'a> {
    pub fn new(registry: &'a TypeRegistry, base_url: &'a str) -> Self {
        Self { registry, base_url }
    }

    /// Get the path for a type name from the registry.
    pub fn get_path(&self, name: &str) -> Option<&str> {
        self.registry.get_path(name)
    }

    /// Convert a Ty to a string with link markers for known types.
    ///
    /// Input:  "dict[str, TaskContext]"
    /// Output: "dict['@link@ /lib/str @@ str @link@', '@link@ /lib/task_context @@ TaskContext @link@']"
    ///
    /// NOTE: Single quotes are required around link markers so they are treated as string
    /// literals by the Python syntax highlighter and not parsed as operators.
    pub fn linkify(&self, ty: &Ty) -> String {
        let ty_str = ty.to_string();
        self.linkify_str(&ty_str)
    }

    /// Convert a type string with link markers for known types.
    /// Quotes are added around links to survive Python syntax highlighting.
    pub fn linkify_str(&self, ty_str: &str) -> String {
        QUALIFIED_TYPE_REGEX
            .replace_all(ty_str, |caps: &regex::Captures| {
                let full_name = &caps[1];

                // First try the full qualified name (for types registered with their full path)
                if let Some(path) = self.registry.get_path(full_name) {
                    return self.link_marker(path, full_name);
                }

                // For qualified names like "module.Type", link each component separately
                if full_name.contains('.') {
                    return self.linkify_namespaced(full_name);
                }

                // Try simple name (no dots)
                if let Some(path) = self.registry.get_path(full_name) {
                    self.link_marker(path, full_name)
                } else {
                    full_name.to_string()
                }
            })
            .into_owned()
    }

    /// Build a single `@link@` marker honoring the configured `base_url`.
    fn link_marker(&self, path: &str, label: &str) -> String {
        format!("'@link@ {}/{path} @@ {label} @link@'", self.base_url)
    }

    /// Handle namespaced types with separate links for each component.
    ///
    /// Input:  "std.Env"
    /// Output: "'@link@ /types/std @@ std @link@'.'@link@ /types/std/env @@ Env @link@'"
    fn linkify_namespaced(&self, type_name: &str) -> String {
        let parts: Vec<&str> = type_name.split('.').collect();
        let mut result = Vec::new();
        let mut path_so_far = String::new();

        for (i, part) in parts.iter().enumerate() {
            let snake_part = to_snake_case(part);

            // Build path progressively
            if i == 0 {
                // First part - look up in registry to get the base path
                if let Some(base_path) = self.registry.get_path(part) {
                    path_so_far = base_path.to_string();
                } else {
                    // If first part not in registry, try with types/ prefix
                    path_so_far = format!("types/{}", snake_part);
                }
                result.push(self.link_marker(&path_so_far, part));
            } else {
                // For subsequent parts, first check if registered in registry
                // (handles properties/functions that are documented on parent module's page)
                if let Some(registered_path) = self.registry.get_path(part) {
                    path_so_far = registered_path.to_string();
                } else {
                    // Otherwise extend the path progressively
                    path_so_far = format!("{}/{}", path_so_far, snake_part);
                }
                result.push(self.link_marker(&path_so_far, part));
            }
        }

        result.join(".")
    }
}

/// Convert CamelCase or PascalCase to snake_case.
fn to_snake_case(s: &str) -> String {
    let mut result = String::new();

    for c in s.chars() {
        if c.is_ascii_uppercase() {
            if !result.is_empty() && result.chars().last().unwrap() != '_' {
                result.push('_');
            }
            result.push(c.to_ascii_lowercase());
        } else {
            result.push(c);
        }
    }
    result
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_linkify_simple_type() {
        let mut registry = TypeRegistry::new();
        registry.register("str", "types/str");
        registry.register("int", "types/int");

        let linker = TypeLinker::new(&registry, "");

        assert_eq!(
            linker.linkify_str("str"),
            "'@link@ /types/str @@ str @link@'"
        );
        assert_eq!(
            linker.linkify_str("int"),
            "'@link@ /types/int @@ int @link@'"
        );
    }

    #[test]
    fn test_linkify_with_base_url() {
        let mut registry = TypeRegistry::new();
        registry.register("str", "types/str");

        let linker = TypeLinker::new(&registry, "/docs");
        assert_eq!(
            linker.linkify_str("str"),
            "'@link@ /docs/types/str @@ str @link@'"
        );
    }

    #[test]
    fn test_linkify_generic_type() {
        let mut registry = TypeRegistry::new();
        registry.register("str", "types/str");
        registry.register("TaskContext", "types/task_context");

        let linker = TypeLinker::new(&registry, "");

        let result = linker.linkify_str("dict[str, TaskContext]");
        assert!(result.contains("'@link@ /types/str @@ str @link@'"));
        assert!(result.contains("'@link@ /types/task_context @@ TaskContext @link@'"));
    }

    #[test]
    fn test_linkify_namespaced_type() {
        let mut registry = TypeRegistry::new();
        registry.register("args", "types/args");
        registry.register("std", "types/std");

        let linker = TypeLinker::new(&registry, "");

        // For "args.TaskArg", each component gets its own link
        let result = linker.linkify_str("args.TaskArg");
        assert_eq!(
            result,
            "'@link@ /types/args @@ args @link@'.'@link@ /types/args/task_arg @@ TaskArg @link@'"
        );

        // For "std.Env", each component gets its own link
        let result = linker.linkify_str("std.Env");
        assert_eq!(
            result,
            "'@link@ /types/std @@ std @link@'.'@link@ /types/std/env @@ Env @link@'"
        );
    }

    #[test]
    fn test_linkify_unknown_type() {
        let registry = TypeRegistry::new();
        let linker = TypeLinker::new(&registry, "");

        // Unknown types should pass through unchanged
        assert_eq!(linker.linkify_str("UnknownType"), "UnknownType");
    }

    #[test]
    fn test_to_snake_case() {
        assert_eq!(to_snake_case("TaskContext"), "task_context");
        assert_eq!(to_snake_case("Env"), "env");
        assert_eq!(to_snake_case("TaskArg"), "task_arg");
        assert_eq!(to_snake_case("args"), "args");
    }
}
