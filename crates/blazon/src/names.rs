//! Rust names for Java members and classes.

use heck::ToSnakeCase;

const KEYWORDS: &[&str] = &[
    "as", "break", "const", "continue", "crate", "else", "enum", "extern", "false", "fn", "for",
    "if", "impl", "in", "let", "loop", "match", "mod", "move", "mut", "pub", "ref", "return",
    "self", "Self", "static", "struct", "super", "trait", "true", "type", "unsafe", "use", "where",
    "while", "async", "await", "dyn", "abstract", "become", "box", "do", "final", "macro",
    "override", "priv", "typeof", "unsized", "virtual", "yield", "try", "gen",
];

/// Words that cannot be raw identifiers either.
const NOT_RAW: &[&str] = &["self", "Self", "super", "crate"];

/// A Rust identifier for `name`: a keyword becomes a raw identifier, and the
/// four that cannot be raw get a trailing underscore.
pub fn ident(name: &str) -> String {
    if NOT_RAW.contains(&name) {
        format!("{name}_")
    } else if KEYWORDS.contains(&name) {
        format!("r#{name}")
    } else {
        name.to_string()
    }
}

/// The name without the `r#` of a raw identifier, for building other names.
pub fn bare(name: &str) -> &str {
    name.strip_prefix("r#").unwrap_or(name)
}

pub fn snake(java: &str) -> String {
    java.to_snake_case()
}

/// Whether `getFoo` reads as the property `foo`: a getter that takes nothing and gives something.
fn is_plain_getter(java: &str, params: usize, returns_value: bool) -> bool {
    params == 0
        && returns_value
        && java
            .strip_prefix("get")
            .and_then(|rest| rest.chars().next())
            .is_some_and(|c| c.is_uppercase())
}

/// The method's Rust name: snake case, with `get_` dropped from a plain
/// accessor (`getWorkspace` is `workspace`); `set_`, `is_` and `has_` stay.
pub fn method(java: &str, params: usize, returns_value: bool) -> String {
    let base = if is_plain_getter(java, params, returns_value) {
        snake(&java[3..])
    } else {
        snake(java)
    };
    ident(&base)
}

/// The name with `get_` kept, for when the short one is taken.
pub fn method_long(java: &str) -> String {
    ident(&snake(java))
}

/// A class's default Rust name: the simple name, inner classes by their own name.
pub fn class(java: &str) -> String {
    java.rsplit(['.', '$']).next().unwrap_or(java).to_string()
}

/// `Outer$Inner` as `OuterInner`, for classes bound on the spec's behalf.
pub fn class_qualified(internal: &str) -> String {
    internal
        .rsplit('/')
        .next()
        .unwrap_or(internal)
        .replace('$', "")
}

/// Starlark's HTML doc strings as plain text for a doc comment: tags
/// dropped, entities decoded, whitespace collapsed, brackets escaped.
pub fn plain_doc(html: &str) -> String {
    let mut text = String::new();
    let mut in_tag = false;
    for c in html.chars() {
        match c {
            '<' => in_tag = true,
            '>' if in_tag => {
                in_tag = false;
                text.push(' ');
            }
            _ if in_tag => {}
            _ => text.push(c),
        }
    }
    let text = text
        .replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&amp;", "&")
        .replace("&quot;", "\"");
    let collapsed = text.split_whitespace().collect::<Vec<_>>().join(" ");
    let mut tidy = collapsed;
    for p in [".", ",", ";", ":", ")"] {
        tidy = tidy.replace(&format!(" {p}"), p);
    }
    tidy = tidy.replace("( ", "(");
    tidy.replace('[', "\\[").replace(']', "\\]")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn accessors_drop_get() {
        assert_eq!(method("getWorkspace", 0, true), "workspace");
        assert_eq!(method("getURL", 0, true), "url");
        assert_eq!(method("getOutputBase", 0, true), "output_base");
    }

    #[test]
    fn other_prefixes_stay() {
        assert_eq!(method("isEmpty", 0, true), "is_empty");
        assert_eq!(method("hasNext", 0, true), "has_next");
        assert_eq!(method("setClock", 1, false), "set_clock");
        assert_eq!(method("getPackage", 2, true), "get_package");
        assert_eq!(method("get", 0, true), "get");
        assert_eq!(method("getaway", 0, true), "getaway");
    }

    #[test]
    fn keywords() {
        assert_eq!(method("type", 0, true), "r#type");
        assert_eq!(method("self", 0, true), "self_");
        assert_eq!(method("getType", 0, true), "r#type");
    }

    #[test]
    fn docs_are_plain() {
        assert_eq!(
            plain_doc(
                "The <code>x</code> of a <a href=\"#y\">label</a> &amp; [more] (<i>see</i>)."
            ),
            "The x of a label & \\[more\\] (see)."
        );
    }

    #[test]
    fn classes() {
        assert_eq!(class("com.x.Outer$Inner"), "Inner");
        assert_eq!(class_qualified("com/x/Outer$Inner"), "OuterInner");
    }
}
