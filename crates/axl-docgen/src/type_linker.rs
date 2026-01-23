use crate::type_registry::TypeRegistry;
use regex::Regex;
use starlark::typing::Ty;
use std::sync::LazyLock;

/// Converts type representations to linkable strings using the `@link@` format
/// that highlight.rs expects.
pub struct TypeLinker<'a> {
    registry: &'a TypeRegistry,
}

// Match qualified names like "module.Type" or simple identifiers
static QUALIFIED_TYPE_REGEX: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"\b([A-Za-z_][A-Za-z0-9_]*(?:\.[A-Za-z_][A-Za-z0-9_]*)*)").unwrap()
});

impl<'a> TypeLinker<'a> {
    pub fn new(registry: &'a TypeRegistry) -> Self {
        Self { registry }
    }

    /// Convert a Ty to a string with link markers for known types.
    ///
    /// Input:  "dict[str, TaskContext]"
    /// Output: "dict['@link@ /lib/str @@ str @link@', '@link@ /lib/task_context @@ TaskContext @link@']"
    pub fn linkify(&self, ty: &Ty) -> String {
        let ty_str = ty.to_string();
        self.linkify_str(&ty_str)
    }

    /// Convert a type string with link markers for known types.
    pub fn linkify_str(&self, ty_str: &str) -> String {
        QUALIFIED_TYPE_REGEX
            .replace_all(ty_str, |caps: &regex::Captures| {
                let full_name = &caps[1];

                // First try the full qualified name
                if let Some(path) = self.registry.get_path(full_name) {
                    return format!("'@link@ /{path} @@ {full_name} @link@'");
                }

                // For qualified names like "module.Type", try to link to the module
                if full_name.contains('.') {
                    let parts: Vec<&str> = full_name.split('.').collect();
                    // Try progressively shorter prefixes
                    for i in (1..parts.len()).rev() {
                        let prefix = parts[..i].join(".");
                        if let Some(path) = self.registry.get_path(&prefix) {
                            return format!("'@link@ /{path} @@ {full_name} @link@'");
                        }
                    }
                    // Try just the first component as the module
                    if let Some(path) = self.registry.get_path(parts[0]) {
                        return format!("'@link@ /{path} @@ {full_name} @link@'");
                    }
                }

                // Try simple name (no dots)
                if let Some(path) = self.registry.get_path(full_name) {
                    format!("'@link@ /{path} @@ {full_name} @link@'")
                } else {
                    full_name.to_string()
                }
            })
            .into_owned()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_linkify_simple_type() {
        let mut registry = TypeRegistry::new();
        registry.register("str", "lib/str");
        registry.register("int", "lib/int");

        let linker = TypeLinker::new(&registry);

        assert_eq!(linker.linkify_str("str"), "'@link@ /lib/str @@ str @link@'");
        assert_eq!(linker.linkify_str("int"), "'@link@ /lib/int @@ int @link@'");
    }

    #[test]
    fn test_linkify_generic_type() {
        let mut registry = TypeRegistry::new();
        registry.register("str", "lib/str");
        registry.register("TaskContext", "lib/task_context");

        let linker = TypeLinker::new(&registry);

        let result = linker.linkify_str("dict[str, TaskContext]");
        assert!(result.contains("'@link@ /lib/str @@ str @link@'"));
        assert!(result.contains("'@link@ /lib/task_context @@ TaskContext @link@'"));
    }

    #[test]
    fn test_linkify_qualified_type() {
        let mut registry = TypeRegistry::new();
        registry.register("args", "lib/args");

        let linker = TypeLinker::new(&registry);

        // For "args.TaskArg", should link to /lib/args with display "args.TaskArg"
        let result = linker.linkify_str("args.TaskArg");
        assert_eq!(result, "'@link@ /lib/args @@ args.TaskArg @link@'");
    }

    #[test]
    fn test_linkify_unknown_type() {
        let registry = TypeRegistry::new();
        let linker = TypeLinker::new(&registry);

        // Unknown types should pass through unchanged
        assert_eq!(linker.linkify_str("UnknownType"), "UnknownType");
    }
}
