use anyhow::{Context, Result, bail};
use serde::Deserialize;
use std::path::Path;

#[derive(Deserialize)]
pub struct Spec {
    /// Bazel versions to bind when the caller names none, e.g. `["9.0.1"]`.
    #[serde(default)]
    pub versions: Vec<String>,
    /// Bind Starlark types for every class (see `ClassSpec::starlark`).
    #[serde(default)]
    pub starlark: bool,
    #[serde(rename = "class")]
    pub classes: Vec<ClassSpec>,
}

#[derive(Deserialize)]
pub struct ClassSpec {
    /// Dotted Java name, e.g. `com.google.devtools.build.lib.bazel.Bazel`.
    pub java: String,
    /// Rust struct name; defaults to the simple class name.
    pub rust: Option<String>,
    /// `name`, `name(<param descriptors>)` to pick an overload, `<init>(...)` for
    /// a constructor, each optionally followed by ` as rust_name` and by `?` when
    /// the member may return null; a leading `.` picks the field when a method has the
    /// same name.
    #[serde(default)]
    pub members: Vec<String>,
    /// Type parameter names, checked against the class file's `Signature`; also
    /// how a class that is in no jar (so opaque, with no members) gets generics.
    #[serde(default)]
    pub type_params: Vec<String>,
    /// Overrides the top-level `starlark`: emit (or not) a Starlark value type
    /// for this class, built from its `@StarlarkMethod`s and the spec's members.
    pub starlark: Option<bool>,
}

pub struct MemberSel {
    pub name: String,
    /// The parameter descriptors, to pick one overload.
    pub params: Option<String>,
    pub rust: Option<String>,
    /// A trailing `?`: the member may return null, so it returns `Option`.
    pub nullable: bool,
    /// A leading `.`: the field of that name, when a method has the same name.
    pub field_only: bool,
    /// `nullable(0,2)`: parameters (by position) that accept null although the class
    /// file does not say so; they become `Option<&_>`.
    pub nullable_params: Vec<usize>,
    /// A leading `@`: the Starlark type exposes the member as an attribute, not a method.
    pub attribute: bool,
    pub text: String,
}

pub fn load(path: &Path) -> Result<Spec> {
    let text = std::fs::read_to_string(path).with_context(|| format!("read {}", path.display()))?;
    let spec: Spec = toml::from_str(&text).with_context(|| format!("parse {}", path.display()))?;
    for c in &spec.classes {
        for m in &c.members {
            parse_member(m)?;
        }
    }
    Ok(spec)
}

pub fn parse_member(text: &str) -> Result<MemberSel> {
    let (body, nullable) = match text.trim().strip_suffix('?') {
        Some(b) => (b, true),
        None => (text.trim(), false),
    };
    let (body, nullable_params) = match body.split_once(" nullable(") {
        Some((before, rest)) => {
            let Some((list, after)) = rest.split_once(')') else {
                bail!("member {text:?}: missing closing parenthesis in nullable(...)");
            };
            let mut indexes = Vec::new();
            for i in list.split(',').map(str::trim).filter(|i| !i.is_empty()) {
                let Ok(i) = i.parse::<usize>() else {
                    bail!("member {text:?}: nullable({list}) takes parameter positions from 0");
                };
                indexes.push(i);
            }
            (format!("{before}{after}"), indexes)
        }
        None => (body.to_string(), Vec::new()),
    };
    let body = body.as_str();
    let (body, attribute) = match body.trim().strip_prefix('@') {
        Some(rest) => (rest, true),
        None => (body.trim(), false),
    };
    let (sel, rust) = match body.split_once(" as ") {
        Some((s, r)) => (s.trim(), Some(r.trim().to_string())),
        None => (body.trim(), None),
    };
    let (sel, field_only) = match sel.strip_prefix('.') {
        Some(rest) => (rest, true),
        None => (sel, false),
    };
    let (name, params) = match sel.split_once('(') {
        Some((n, rest)) => {
            let Some(p) = rest.strip_suffix(')') else {
                bail!("member {text:?}: missing closing parenthesis");
            };
            (n.to_string(), Some(p.to_string()))
        }
        None => (sel.to_string(), None),
    };
    if name.is_empty() {
        bail!("member {text:?}: empty name");
    }
    Ok(MemberSel {
        name,
        params,
        rust,
        nullable,
        field_only,
        nullable_params,
        attribute,
        text: text.to_string(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn plain_names() {
        let m = parse_member("getWorkspace").expect("member");
        assert_eq!(m.name, "getWorkspace");
        assert!(m.params.is_none() && m.rust.is_none() && !m.nullable && !m.field_only);
    }

    #[test]
    fn overloads_aliases_and_nullability() {
        let m = parse_member("put(Ljava/lang/Object;I) as put_at?").expect("member");
        assert_eq!(m.name, "put");
        assert_eq!(m.params.as_deref(), Some("Ljava/lang/Object;I"));
        assert_eq!(m.rust.as_deref(), Some("put_at"));
        assert!(m.nullable);
    }

    #[test]
    fn nullable_parameters_and_attributes() {
        let m = parse_member("@getName nullable(1, 3) as name?").expect("member");
        assert_eq!(m.name, "getName");
        assert_eq!(m.nullable_params, [1, 3]);
        assert_eq!(m.rust.as_deref(), Some("name"));
        assert!(m.attribute && m.nullable);
        assert!(parse_member("x nullable(a)").is_err());
        assert!(parse_member("x nullable(1").is_err());
    }

    #[test]
    fn constructors_and_fields() {
        let c = parse_member("<init>() as with_defaults").expect("member");
        assert_eq!(c.name, "<init>");
        assert_eq!(c.params.as_deref(), Some(""));
        let f = parse_member(".held").expect("member");
        assert!(f.field_only);
        assert_eq!(f.name, "held");
    }

    #[test]
    fn malformed_members() {
        assert!(parse_member("put(I").is_err());
        assert!(parse_member("").is_err());
        assert!(parse_member("(I)").is_err());
    }

    #[test]
    fn class_entries() {
        let spec: Spec = toml::from_str(
            r#"
versions = ["9.0.1"]
starlark = true

[[class]]
java = "a.B"
rust = "Bee"
members = ["x"]
starlark = false
"#,
        )
        .expect("spec");
        assert_eq!(spec.versions, ["9.0.1"]);
        assert!(spec.starlark);
        assert_eq!(spec.classes[0].rust.as_deref(), Some("Bee"));
        assert_eq!(spec.classes[0].starlark, Some(false));
    }
}
