//! `ctx.config(**settings)`: typed settings named like Bazel's options without
//! the dashes, validated against the option schema generated from the release
//! and converted to the option strings the engine's own `OptionsParser` reads.
//! The strings never reach a script: errors, warnings and help are reworded to
//! name the setting.

use crate::bindings::{self, v9_0_1};
use anyhow::{Result, bail};
use blazon_rt::options::{Kind, OptionDef};

/// A setting's value as the script gave it.
#[derive(Clone, Debug, PartialEq)]
pub enum Value {
    Bool(bool),
    Int(i64),
    Float(f64),
    Str(String),
    List(Vec<Value>),
    Dict(Vec<(String, Value)>),
}

impl Value {
    fn type_name(&self) -> &'static str {
        match self {
            Value::Bool(_) => "bool",
            Value::Int(_) => "int",
            Value::Float(_) => "float",
            Value::Str(_) => "string",
            Value::List(_) => "list",
            Value::Dict(_) => "dict",
        }
    }

    pub fn repr(&self) -> String {
        match self {
            Value::Bool(b) => if *b { "True" } else { "False" }.to_string(),
            Value::Int(i) => i.to_string(),
            Value::Float(f) => f.to_string(),
            Value::Str(s) => format!("{s:?}"),
            Value::List(l) => format!(
                "[{}]",
                l.iter().map(Value::repr).collect::<Vec<_>>().join(", ")
            ),
            Value::Dict(d) => format!(
                "{{{}}}",
                d.iter()
                    .map(|(k, v)| format!("{k:?}: {}", v.repr()))
                    .collect::<Vec<_>>()
                    .join(", ")
            ),
        }
    }
}

/// The schema of the Bazel release this build is bound to.
pub fn schema() -> &'static [OptionDef] {
    bindings::options(v9_0_1::BAZEL_VERSION).unwrap_or(&[])
}

/// The option a setting names, a current or a former name; with the former
/// name, a warning to print.
pub fn lookup(name: &str) -> Result<(&'static OptionDef, Option<String>)> {
    let table = schema();
    if let Some(def) = blazon_rt::options::find(table, name) {
        let warning =
            (def.name != name).then(|| format!("config setting `{name}` is now `{}`", def.name));
        return Ok((def, warning));
    }
    let close = blazon_rt::options::closest(table, name);
    if close.is_empty() {
        bail!("unknown config setting `{name}`");
    }
    bail!(
        "unknown config setting `{name}`; did you mean {}?",
        close
            .iter()
            .map(|c| format!("`{c}`"))
            .collect::<Vec<_>>()
            .join(", ")
    );
}

/// The warning a deprecated setting carries, if any.
pub fn deprecation(def: &OptionDef) -> Option<String> {
    def.deprecated.then(|| {
        let why = strip_dashes(def.deprecation_warning.trim());
        if why.is_empty() {
            format!("config setting `{}` is deprecated", def.name)
        } else {
            format!("config setting `{}` is deprecated: {why}", def.name)
        }
    })
}

/// What a kind reads like in an error.
pub fn describe_kind(kind: &Kind) -> String {
    match kind {
        Kind::Bool => "a bool".into(),
        Kind::Int | Kind::Long => "an int".into(),
        Kind::Double => "a number".into(),
        Kind::String => "a string".into(),
        Kind::Label => "a label string".into(),
        Kind::Path => "a path string".into(),
        Kind::Duration => "a duration string such as \"30s\"".into(),
        Kind::Void => "True".into(),
        Kind::Enum(values) => format!(
            "one of {}",
            values
                .iter()
                .map(|v| format!("{:?}", v.to_lowercase()))
                .collect::<Vec<_>>()
                .join(", ")
        ),
        Kind::List(inner) => format!("a list of {}", plural(inner)),
        Kind::Dict(inner) => format!("a dict of strings to {}", plural(inner)),
        Kind::Opaque(_) => "a string".into(),
    }
}

fn plural(kind: &Kind) -> String {
    match kind {
        Kind::Bool => "bools".into(),
        Kind::Int | Kind::Long => "ints".into(),
        Kind::Double => "numbers".into(),
        Kind::String | Kind::Opaque(_) => "strings".into(),
        Kind::Label => "label strings".into(),
        Kind::Path => "path strings".into(),
        other => describe_kind(other),
    }
}

/// The kind as `describe` reports it.
pub fn kind_name(kind: &Kind) -> String {
    match kind {
        Kind::Bool => "bool".into(),
        Kind::Int => "int".into(),
        Kind::Long => "long".into(),
        Kind::Double => "double".into(),
        Kind::String => "string".into(),
        Kind::Label => "label".into(),
        Kind::Path => "path".into(),
        Kind::Duration => "duration".into(),
        Kind::Void => "flag".into(),
        Kind::Enum(values) => format!(
            "enum({})",
            values
                .iter()
                .map(|v| v.to_lowercase())
                .collect::<Vec<_>>()
                .join("|")
        ),
        Kind::List(inner) => format!("list of {}", kind_name(inner)),
        Kind::Dict(inner) => format!("dict of {}", kind_name(inner)),
        Kind::Opaque(_) => "string".into(),
    }
}

fn scalar(def: &OptionDef, kind: &Kind, value: &Value) -> Result<String> {
    let wrong = || {
        anyhow::anyhow!(
            "config setting `{}` expects {}, got {}",
            def.name,
            describe_kind(&def.kind),
            value.type_name()
        )
    };
    Ok(match (kind, value) {
        (Kind::Bool, Value::Bool(b)) => b.to_string(),
        (Kind::Int | Kind::Long, Value::Int(i)) => i.to_string(),
        (Kind::Double, Value::Int(i)) => i.to_string(),
        (Kind::Double, Value::Float(f)) => f.to_string(),
        (
            Kind::String | Kind::Label | Kind::Path | Kind::Duration | Kind::Opaque(_),
            Value::Str(s),
        ) => s.clone(),
        (Kind::Enum(values), Value::Str(s)) => {
            if !values.iter().any(|v| v.eq_ignore_ascii_case(s)) {
                bail!(
                    "config setting `{}` got {s:?}; it expects {}",
                    def.name,
                    describe_kind(kind)
                );
            }
            s.to_lowercase()
        }
        _ => return Err(wrong()),
    })
}

/// The option strings for one setting, handed to the option parser.
pub fn arguments(def: &OptionDef, value: &Value) -> Result<Vec<String>> {
    let name = def.name;
    match &def.kind {
        Kind::Void => match value {
            Value::Bool(true) => Ok(vec![format!("--{name}")]),
            _ => bail!(
                "config setting `{name}` takes only True, got {}",
                value.repr()
            ),
        },
        Kind::List(inner) => {
            let Value::List(items) = value else {
                bail!(
                    "config setting `{name}` expects {}, got {}",
                    describe_kind(&def.kind),
                    value.type_name()
                );
            };
            let each: Vec<String> = items
                .iter()
                .map(|i| scalar(def, inner, i))
                .collect::<Result<_>>()?;
            Ok(if def.allows_multiple {
                each.into_iter().map(|i| format!("--{name}={i}")).collect()
            } else {
                vec![format!("--{name}={}", each.join(","))]
            })
        }
        Kind::Dict(inner) => {
            let Value::Dict(entries) = value else {
                bail!(
                    "config setting `{name}` expects {}, got {}",
                    describe_kind(&def.kind),
                    value.type_name()
                );
            };
            entries
                .iter()
                .map(|(k, v)| {
                    let text = match (&**inner, v) {
                        (Kind::List(item), Value::List(items)) => items
                            .iter()
                            .map(|i| scalar(def, item, i))
                            .collect::<Result<Vec<_>>>()?
                            .join(","),
                        (kind, v) => scalar(def, kind, v)?,
                    };
                    Ok(format!("--{name}={k}={text}"))
                })
                .collect()
        }
        kind => Ok(vec![format!("--{name}={}", scalar(def, kind, value)?)]),
    }
}

/// `text` with every option's leading dashes dropped, so that `--foo` reads
/// `foo`: the schema's help, Bazel's parse errors and warnings.
pub fn strip_dashes(text: &str) -> String {
    let chars: Vec<char> = text.chars().collect();
    let mut out = String::with_capacity(text.len());
    let mut i = 0;
    while i < chars.len() {
        let at_word = i == 0 || !chars[i - 1].is_alphanumeric();
        if at_word
            && chars[i] == '-'
            && chars.get(i + 1) == Some(&'-')
            && chars.get(i + 2).is_some_and(|c| c.is_alphanumeric())
        {
            i += 2;
            continue;
        }
        out.push(chars[i]);
        i += 1;
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn def(name: &str) -> &'static OptionDef {
        lookup(name).unwrap().0
    }

    #[test]
    fn unknown_setting_suggests_close_names() {
        let e = lookup("compilaton_mode").unwrap_err().to_string();
        assert!(e.contains("`compilation_mode`"), "{e}");
    }

    #[test]
    fn values_become_option_strings() {
        let v = Value::List(vec![Value::Str("//a:x".into()), Value::Str("//a:y".into())]);
        assert_eq!(
            arguments(def("platforms"), &v).unwrap(),
            ["--platforms=//a:x,//a:y"]
        );
        let v = Value::List(vec![Value::Str("-O3".into()), Value::Str("-g".into())]);
        assert_eq!(
            arguments(def("copt"), &v).unwrap(),
            ["--copt=-O3", "--copt=-g"]
        );
        let v = Value::Dict(vec![("A".into(), Value::Str("b".into()))]);
        assert_eq!(
            arguments(def("action_env"), &v).unwrap(),
            ["--action_env=A=b"]
        );
        assert_eq!(
            arguments(def("compilation_mode"), &Value::Str("OPT".into())).unwrap(),
            ["--compilation_mode=opt"]
        );
    }

    #[test]
    fn wrong_types_name_the_setting() {
        let e = arguments(def("copt"), &Value::Str("x".into()))
            .unwrap_err()
            .to_string();
        assert!(
            e.contains("`copt` expects a list of strings, got string"),
            "{e}"
        );
        let e = arguments(def("compilation_mode"), &Value::Str("turbo".into()))
            .unwrap_err()
            .to_string();
        assert!(e.contains("\"fastbuild\""), "{e}");
        assert!(!e.contains("--"), "{e}");
    }

    #[test]
    fn dashes_are_stripped() {
        assert_eq!(
            strip_dashes("use `--foo=bar` or --baz; a -- b; x--y"),
            "use `foo=bar` or baz; a -- b; x--y"
        );
    }
}
