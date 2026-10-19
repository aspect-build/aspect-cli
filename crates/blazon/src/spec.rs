use anyhow::{Context, Result, bail};
use serde::Deserialize;
use std::path::Path;

#[derive(Clone, Deserialize)]
pub struct Spec {
    /// Bazel versions to bind when the caller names none, e.g. `["9.0.1"]`.
    #[serde(default)]
    pub versions: Vec<String>,
    /// Bind Starlark types for every class (see `ClassSpec::starlark`).
    #[serde(default)]
    pub starlark: bool,
    #[serde(rename = "class", default)]
    pub classes: Vec<ClassSpec>,
    /// Generate Bazel's option schema; absent means none.
    pub options: Option<OptionsSpec>,
    /// Java interfaces and abstract classes Rust implements.
    #[serde(rename = "implement", default)]
    pub implementations: Vec<ImplementSpec>,
}

/// A Java interface (or an abstract class) that Rust implements: blazon
/// generates a trait with one method per abstract method, and a class file
/// whose methods are native and call it.
#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ImplementSpec {
    /// Dotted name of the interface, or of the abstract class to extend.
    pub java: String,
    /// The trait's name; defaults to the handle's name followed by `Impl`.
    pub rust: Option<String>,
    /// Which abstract methods the trait has, by `name`, `name(<descriptors>)`, each
    /// optionally ` as rust_name` and `?` for a result that may be null; all of
    /// them when empty. The ones left out throw `AbstractMethodError` when called.
    #[serde(default)]
    pub members: Vec<String>,
    /// A class the implementation extends, for an interface that needs one: it
    /// must have a public or protected no-argument constructor.
    pub extends: Option<String>,
    /// The Bazel versions this is implemented in; every version when empty.
    #[serde(default)]
    pub versions: Vec<String>,
}

/// Which of Bazel's options go in the generated schema. An option is included
/// by the first rule below that decides it:
///
/// A class in `include_classes` or `exclude_classes` is a dotted name, or a
/// prefix ending in `*`.
///
/// 1. its name is in `exclude`: out;
/// 2. its name is in `include`: in;
/// 3. its class is in `exclude_classes`: out;
/// 4. it has a metadata tag in `exclude_metadata_tags` (default `INTERNAL`, which
///    is what keeps options with no command-line spelling out): out;
/// 5. its class is in `include_classes`: in;
/// 6. `fragments` and its class is a configuration fragment (extends
///    `FragmentOptions`): in;
/// 7. it has an effect tag in `include_effect_tags` and not only tags in
///    `exclude_effect_tags`: in;
/// 8. out.
#[derive(Clone, Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct OptionsSpec {
    #[serde(default)]
    pub fragments: bool,
    #[serde(default)]
    pub include_effect_tags: Vec<String>,
    #[serde(default)]
    pub exclude_effect_tags: Vec<String>,
    #[serde(default = "internal_only")]
    pub exclude_metadata_tags: Vec<String>,
    #[serde(default)]
    pub include_classes: Vec<String>,
    #[serde(default)]
    pub exclude_classes: Vec<String>,
    #[serde(default)]
    pub include: Vec<String>,
    #[serde(default)]
    pub exclude: Vec<String>,
}

fn internal_only() -> Vec<String> {
    vec!["INTERNAL".to_string()]
}

#[derive(Clone, Deserialize)]
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
    /// The Bazel versions this class is bound in; every version when empty.
    #[serde(default)]
    pub versions: Vec<String>,
    /// Members left out of this version by a ` when` condition, so that the
    /// shape report can tell them from members that went missing.
    #[serde(skip)]
    pub conditional: Vec<String>,
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
    /// ` when 9.0.1, 9.2.0`: the member exists only in these versions.
    pub versions: Vec<String>,
    /// The text without the `when` condition, which names the member in every version.
    pub text: String,
}

pub fn load(path: &Path) -> Result<Spec> {
    let text = std::fs::read_to_string(path).with_context(|| format!("read {}", path.display()))?;
    let spec: Spec = toml::from_str(&text).with_context(|| format!("parse {}", path.display()))?;
    let mut spec = spec;
    for c in &spec.classes {
        for m in &c.members {
            parse_member(m)?;
        }
    }
    for i in &spec.implementations {
        for m in &i.members {
            parse_member(m)?;
        }
    }
    let wanted: Vec<String> = spec
        .implementations
        .iter()
        .map(|i| i.java.clone())
        .collect();
    for java in wanted {
        let versions: Vec<String> = spec
            .implementations
            .iter()
            .find(|i| i.java == java)
            .map(|i| i.versions.clone())
            .unwrap_or_default();
        let jdk = java.starts_with("java.") || java.starts_with("javax.");
        if !jdk && !spec.classes.iter().any(|c| c.java == java) {
            spec.classes.push(ClassSpec {
                java,
                rust: None,
                members: Vec::new(),
                type_params: Vec::new(),
                starlark: Some(false),
                versions,
                conditional: Vec::new(),
            });
        }
    }
    Ok(spec)
}

impl Spec {
    /// The spec as it applies to one Bazel version: classes and implementations
    /// whose `versions` leave it out are dropped, and so are members whose `when`
    /// does (remembered in `conditional`).
    pub fn for_version(&self, label: &str) -> Spec {
        let applies = |v: &[String]| v.is_empty() || v.iter().any(|x| x == label);
        let mut out = self.clone();
        out.classes.retain(|c| applies(&c.versions));
        out.implementations.retain(|i| applies(&i.versions));
        for c in &mut out.classes {
            let (keep, skip): (Vec<String>, Vec<String>) = std::mem::take(&mut c.members)
                .into_iter()
                .partition(|m| parse_member(m).is_ok_and(|s| applies(&s.versions)));
            c.members = keep;
            c.conditional = skip
                .iter()
                .filter_map(|m| parse_member(m).ok().map(|s| s.text))
                .collect();
        }
        for i in &mut out.implementations {
            i.members
                .retain(|m| parse_member(m).is_ok_and(|s| applies(&s.versions)));
        }
        out
    }
}

pub fn parse_member(text: &str) -> Result<MemberSel> {
    let (text, versions) = match text.rsplit_once(" when ") {
        Some((member, list)) => {
            let versions: Vec<String> = list
                .split(',')
                .map(|v| v.trim().to_string())
                .filter(|v| !v.is_empty())
                .collect();
            if versions.is_empty() {
                bail!("member {text:?}: `when` needs Bazel versions, e.g. `when 9.0.1, 9.2.0`");
            }
            (member.trim(), versions)
        }
        None => (text.trim(), Vec::new()),
    };
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
        versions,
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
