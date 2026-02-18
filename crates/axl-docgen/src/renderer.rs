use crate::traversal::{DocPage, DocPageItem};
use crate::type_linker::TypeLinker;
use starlark::docs::{DocFunction, DocParam, DocProperty};

const PRELUDE: &str = r#"<pre class="language-python"><code>"#;
const POSTLUDE: &str = r#"</code></pre>"#;

/// Renders documentation pages to markdown format.
pub struct Renderer<'a> {
    linker: &'a TypeLinker<'a>,
}

impl<'a> Renderer<'a> {
    pub fn new(linker: &'a TypeLinker<'a>) -> Self {
        Self { linker }
    }

    /// Render a complete documentation page to markdown.
    pub fn render_page(&self, page: &DocPage) -> String {
        let mut output = String::new();

        // Add a placeholder heading that will be removed by main.rs
        output.push_str("# Placeholder\n\n");

        // Separate items by category for ordered rendering
        let mut types = Vec::new();
        let mut modules = Vec::new();
        let mut functions = Vec::new();
        let mut properties = Vec::new();

        for item in page.items.iter() {
            match item {
                DocPageItem::Type { .. } => types.push(item),
                DocPageItem::Module { .. } => modules.push(item),
                DocPageItem::Function { .. } => functions.push(item),
                DocPageItem::Property { .. } => properties.push(item),
            }
        }

        // Render types first (no separators)
        for item in &types {
            if let DocPageItem::Type { name, docs } = item {
                output.push_str(&self.render_type(name, docs.as_ref()));
            }
        }

        // Render modules (no separators)
        for item in &modules {
            if let DocPageItem::Module { name, docs } = item {
                output.push_str(&self.render_module(name, docs.as_ref()));
            }
        }

        // Render functions (no separators, with expandable docs)
        for item in &functions {
            if let DocPageItem::Function {
                name,
                parent_type,
                func,
            } = item
            {
                output.push_str(&self.render_function(name, parent_type.as_deref(), func));
            }
        }

        // Render properties (no separators, with expandable docs)
        for item in &properties {
            if let DocPageItem::Property {
                name,
                parent_type,
                prop,
            } = item
            {
                output.push_str(&self.render_property(name, parent_type.as_deref(), prop));
            }
        }

        output
    }

    fn render_function(&self, name: &str, parent: Option<&str>, func: &DocFunction) -> String {
        let mut output = String::new();

        // Display name for the function
        let display_name = if let Some(p) = parent {
            format!("{}.{}", p, name)
        } else {
            name.to_string()
        };

        // Render as: `function` **name**
        output.push_str(&format!(
            "`function` **{}**\n\n",
            escape_underscores(&display_name)
        ));

        // Code block with signature
        output.push_str(PRELUDE);
        output.push_str(&self.render_function_signature(name, parent, func));
        output.push_str(POSTLUDE);
        output.push_str("\n\n");

        // Documentation
        if let Some(docs) = &func.docs {
            output.push_str(&render_docstring(docs));
        }

        // Parameter documentation
        if let Some(params_md) = render_param_docs(func) {
            output.push_str("**Parameters**\n\n");
            output.push_str(&params_md);
            output.push('\n');
        }

        output
    }

    fn render_property(&self, name: &str, parent: Option<&str>, prop: &DocProperty) -> String {
        let mut output = String::new();

        // Display name for the property
        let display_name = if let Some(p) = parent {
            format!("{}.{}", p, name)
        } else {
            name.to_string()
        };

        // Render as: `property` **name**
        output.push_str(&format!(
            "`property` **{}**\n\n",
            escape_underscores(&display_name)
        ));

        // Code block with property type
        output.push_str(PRELUDE);
        output.push_str(&self.render_property_signature(name, parent, prop));
        output.push_str(POSTLUDE);
        output.push_str("\n\n");

        // Documentation
        if let Some(docs) = &prop.docs {
            output.push_str(&render_docstring(docs));
        }

        output
    }

    fn render_function_signature(
        &self,
        name: &str,
        parent: Option<&str>,
        func: &DocFunction,
    ) -> String {
        let mut sig = String::new();

        // Function header
        sig.push_str("def ");
        if let Some(p) = parent {
            sig.push_str(&format!("{}.", p));
        }
        sig.push_str(name);
        sig.push('(');

        let mut params_rendered = Vec::new();
        let mut needs_newlines = false;

        // Positional-only parameters
        for param in &func.params.pos_only {
            let type_str = self.linker.linkify(&param.typ);
            let default_str = param
                .default_value
                .as_ref()
                .map(|d| format!(" = {}", d))
                .unwrap_or_default();
            params_rendered.push(format!("{}: {}{}", param.name, type_str, default_str));
        }

        // Add "/" separator if there were positional-only params
        if !func.params.pos_only.is_empty() {
            params_rendered.push("/".to_string());
            needs_newlines = true;
        }

        // Positional-or-named parameters
        for param in &func.params.pos_or_named {
            let type_str = self.linker.linkify(&param.typ);
            let default_str = param
                .default_value
                .as_ref()
                .map(|d| format!(" = {}", d))
                .unwrap_or_default();
            params_rendered.push(format!("{}: {}{}", param.name, type_str, default_str));
        }

        // *args or bare * for keyword-only params
        if let Some(args) = &func.params.args {
            let type_str = self.linker.linkify(&args.typ);
            params_rendered.push(format!("*{}: {}", args.name, type_str));
            needs_newlines = true;
        } else if !func.params.named_only.is_empty() {
            params_rendered.push("*".to_string());
            needs_newlines = true;
        }

        // Keyword-only (named-only) parameters
        for param in &func.params.named_only {
            let type_str = self.linker.linkify(&param.typ);
            let default_str = param
                .default_value
                .as_ref()
                .map(|d| format!(" = {}", d))
                .unwrap_or_default();
            params_rendered.push(format!("{}: {}{}", param.name, type_str, default_str));
        }

        // **kwargs
        if let Some(kwargs) = &func.params.kwargs {
            let type_str = self.linker.linkify(&kwargs.typ);
            params_rendered.push(format!("**{}: {}", kwargs.name, type_str));
            needs_newlines = true;
        }

        // Format parameters
        if params_rendered.is_empty() {
            sig.push(')');
        } else if needs_newlines || params_rendered.len() > 2 {
            // Multi-line format
            sig.push('\n');
            for (i, param) in params_rendered.iter().enumerate() {
                sig.push_str("    ");
                sig.push_str(param);
                if i < params_rendered.len() - 1 {
                    sig.push(',');
                }
                sig.push('\n');
            }
            sig.push(')');
        } else {
            // Single-line format
            sig.push_str(&params_rendered.join(", "));
            sig.push(')');
        }

        // Return type
        let return_type = self.linker.linkify(&func.ret.typ);
        sig.push_str(" -> ");
        sig.push_str(&return_type);

        sig
    }

    fn render_property_signature(
        &self,
        name: &str,
        parent: Option<&str>,
        prop: &DocProperty,
    ) -> String {
        let type_str = self.linker.linkify(&prop.typ);

        if let Some(p) = parent {
            format!("{}.{}: {}", p, name, type_str)
        } else {
            format!("{}: {}", name, type_str)
        }
    }

    fn render_type(&self, name: &str, _docs: Option<&starlark::docs::DocString>) -> String {
        // Render as simple line: `type` [TypeName](path)
        if let Some(path) = self.linker.get_path(name) {
            format!("`type` [{}](/{})\n\n", name, path)
        } else {
            format!("`type` {}\n\n", name)
        }
    }

    fn render_module(&self, name: &str, _docs: Option<&starlark::docs::DocString>) -> String {
        // Render as simple line: `module` [name](path)
        if let Some(path) = self.linker.get_path(name) {
            format!("`module` [{}](/{})\n\n", name, path)
        } else {
            format!("`module` {}\n\n", name)
        }
    }
}

/// Render per-parameter documentation as a markdown list.
/// Returns `None` if no parameter has docs attached.
fn render_param_docs(func: &DocFunction) -> Option<String> {
    let p = &func.params;

    // Collect (display_name, DocParam) in declaration order, matching starlark-rust's
    // doc_params_with_starred_names() logic (which is pub(crate)).
    let mut pairs: Vec<(String, &DocParam)> = Vec::new();
    for param in &p.pos_only {
        pairs.push((param.name.clone(), param));
    }
    for param in &p.pos_or_named {
        pairs.push((param.name.clone(), param));
    }
    if let Some(args) = &p.args {
        pairs.push((format!("*{}", args.name), args));
    }
    for param in &p.named_only {
        pairs.push((param.name.clone(), param));
    }
    if let Some(kwargs) = &p.kwargs {
        pairs.push((format!("**{}", kwargs.name), kwargs));
    }

    let mut output = String::new();
    for (display_name, param) in &pairs {
        let Some(docs) = &param.docs else { continue };
        let doc_text = match &docs.details {
            Some(details) => format!("{}\n\n{}", docs.summary, details),
            None => docs.summary.clone(),
        };
        let mut lines = doc_text.lines();
        if let Some(first) = lines.next() {
            output.push_str(&format!("* `{}`: {}\n", display_name, first));
            for line in lines {
                output.push_str(&format!("  {}\n", line));
            }
        }
    }

    if output.is_empty() {
        None
    } else {
        Some(output)
    }
}

/// Escape underscores in markdown headings to prevent interpretation as emphasis.
fn escape_underscores(s: &str) -> String {
    s.replace('_', "\\_")
}

/// Render a DocString to markdown format.
fn render_docstring(docs: &starlark::docs::DocString) -> String {
    let mut output = String::new();

    output.push_str(&docs.summary);
    output.push_str("\n\n");

    if let Some(details) = &docs.details {
        output.push_str(details);
        output.push_str("\n\n");
    }

    output
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_escape_underscores() {
        assert_eq!(escape_underscores("task_context"), "task\\_context");
        assert_eq!(escape_underscores("boolean_list"), "boolean\\_list");
        assert_eq!(escape_underscores("simple"), "simple");
    }
}
