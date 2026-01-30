use std::collections::HashMap;

/// Registry mapping type names to their documentation paths.
///
/// This is built during traversal of the DocModule tree and used
/// during rendering to generate cross-reference links.
#[derive(Debug, Default)]
pub struct TypeRegistry {
    type_to_path: HashMap<String, String>,
}

impl TypeRegistry {
    pub fn new() -> Self {
        Self::default()
    }

    /// Register a type with its documentation path.
    /// e.g., register("TaskContext", "lib/task_context")
    pub fn register(&mut self, type_name: &str, path: &str) {
        self.type_to_path
            .insert(type_name.to_owned(), path.to_owned());
    }

    /// Get the path for a registered type.
    pub fn get_path(&self, type_name: &str) -> Option<&str> {
        self.type_to_path.get(type_name).map(String::as_str)
    }
}
