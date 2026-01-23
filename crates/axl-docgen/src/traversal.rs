use crate::type_registry::TypeRegistry;
use starlark::docs::{DocFunction, DocItem, DocMember, DocModule, DocProperty};
use std::collections::HashMap;

/// Represents a documentation page with all its items.
#[derive(Debug, Clone)]
pub struct DocPage {
    pub path: String,
    pub items: Vec<DocPageItem>,
}

/// An item that appears on a documentation page.
#[derive(Debug, Clone)]
pub enum DocPageItem {
    Function {
        name: String,
        parent_type: Option<String>,
        func: DocFunction,
    },
    Property {
        name: String,
        parent_type: Option<String>,
        prop: DocProperty,
    },
}

/// The result of traversing a DocModule tree.
pub struct TraversalResult {
    pub pages: HashMap<String, DocPage>,
    pub registry: TypeRegistry,
}

/// Traverse a DocModule tree and collect documentation items with their paths.
pub fn traverse(module: &DocModule, base_path: &str) -> TraversalResult {
    let mut registry = TypeRegistry::new();
    let mut pages: HashMap<String, DocPage> = HashMap::new();

    // Register builtin types
    register_builtin_types(&mut registry);

    traverse_module(module, base_path, &mut pages, &mut registry);

    TraversalResult { pages, registry }
}

fn register_builtin_types(registry: &mut TypeRegistry) {
    // Register common builtin types
    registry.register("str", "lib/str");
    registry.register("int", "lib/int");
    registry.register("bool", "lib/bool");
    registry.register("float", "lib/float");
    registry.register("list", "lib/list");
    registry.register("dict", "lib/dict");
    registry.register("tuple", "lib/tuple");
    registry.register("None", "lib/none");
    registry.register("NoneType", "lib/none");
}

fn traverse_module(
    module: &DocModule,
    path: &str,
    pages: &mut HashMap<String, DocPage>,
    registry: &mut TypeRegistry,
) {
    // Collect items that belong on this module's page
    let mut module_items = Vec::new();

    for (name, doc_item) in module.members.iter() {
        match doc_item {
            DocItem::Module(submodule) => {
                // Calculate the submodule path and normalize it
                let submodule_path = normalize_path(&format!("{}/{}", path, to_snake_case(name)));

                // Register the module name for qualified type resolution
                registry.register(name, &submodule_path);
                let snake_name = to_snake_case(name);
                registry.register(&snake_name, &submodule_path);

                // Recurse into submodule
                traverse_module(submodule, &submodule_path, pages, registry);
            }
            DocItem::Type(doc_type) => {
                // Calculate the type path and normalize it
                let type_path = normalize_path(&format!("{}/{}", path, to_snake_case(name)));

                // Register both original and snake_case names
                registry.register(name, &type_path);
                let snake_name = to_snake_case(name);
                registry.register(&snake_name, &type_path);

                // Collect type members for the type's page
                let mut type_items = Vec::new();
                for (member_name, member) in doc_type.members.iter() {
                    match member {
                        DocMember::Function(func) => {
                            type_items.push(DocPageItem::Function {
                                name: member_name.clone(),
                                parent_type: Some(name.clone()),
                                func: func.clone(),
                            });
                        }
                        DocMember::Property(prop) => {
                            type_items.push(DocPageItem::Property {
                                name: member_name.clone(),
                                parent_type: Some(name.clone()),
                                prop: prop.clone(),
                            });
                        }
                    }
                }

                // Create a page for this type (even if empty - the type itself is documentation)
                add_page(pages, type_path, type_items);
            }
            DocItem::Member(DocMember::Function(func)) => {
                // Add function to current module's page
                module_items.push(DocPageItem::Function {
                    name: name.clone(),
                    parent_type: None,
                    func: func.clone(),
                });

                // Also register the function for linking
                let func_path = normalize_path(&format!("{}/{}", path, to_snake_case(name)));
                registry.register(name, &func_path);
                let snake_name = to_snake_case(name);
                registry.register(&snake_name, &func_path);
            }
            DocItem::Member(DocMember::Property(prop)) => {
                // Add property to current module's page
                module_items.push(DocPageItem::Property {
                    name: name.clone(),
                    parent_type: None,
                    prop: prop.clone(),
                });

                // Also register the property for linking
                let prop_path = normalize_path(&format!("{}/{}", path, to_snake_case(name)));
                registry.register(name, &prop_path);
                let snake_name = to_snake_case(name);
                registry.register(&snake_name, &prop_path);
            }
        }
    }

    // Create the module's page with all its direct members
    // Always create a page for modules, even if empty (serves as index)
    let normalized_path = normalize_path(path);
    add_page(pages, normalized_path, module_items);
}

/// Add items to a page, creating it if it doesn't exist or merging if it does.
fn add_page(pages: &mut HashMap<String, DocPage>, path: String, items: Vec<DocPageItem>) {
    if let Some(existing) = pages.get_mut(&path) {
        // Merge items into existing page
        existing.items.extend(items);
    } else {
        // Create new page
        pages.insert(path.clone(), DocPage { path, items });
    }
}

/// Normalize a path to handle index modules.
/// E.g., "lib/std/std" becomes "lib/std" since we don't want separate index pages.
fn normalize_path(path: &str) -> String {
    let parts: Vec<&str> = path.split('/').collect();
    let snake_parts: Vec<String> = parts.iter().map(|p| to_snake_case(p)).collect();

    if path != "lib" && snake_parts.len() > 2 {
        if let (Some(last), Some(second_last)) =
            (snake_parts.last(), snake_parts.get(snake_parts.len() - 2))
        {
            if last == second_last {
                return snake_parts[..snake_parts.len() - 1].join("/");
            }
        }
    }

    snake_parts.join("/")
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
    fn test_to_snake_case() {
        assert_eq!(to_snake_case("TaskContext"), "task_context");
        assert_eq!(to_snake_case("BuildEvent"), "build_event");
        assert_eq!(to_snake_case("args"), "args");
        assert_eq!(to_snake_case("XMLParser"), "x_m_l_parser");
    }

    #[test]
    fn test_normalize_path() {
        assert_eq!(normalize_path("lib/std/std"), "lib/std");
        assert_eq!(normalize_path("lib/std"), "lib/std");
        assert_eq!(normalize_path("lib"), "lib");
        assert_eq!(
            normalize_path("lib/bazel/build/BuildEvent"),
            "lib/bazel/build/build_event"
        );
    }
}
