//! Bazel's Starlark annotations (`@StarlarkBuiltin`, `@StarlarkMethod`,
//! `@Param`, `@ParamType`) read from class files, and the Starlark interfaces
//! a class implements. Annotation defaults are those declared in
//! `net/starlark/java/annot` in Bazel 9.0.1.

use crate::signature::{self, Ty};
use cafebabe::attributes::{Annotation, AnnotationElementValue};

pub(crate) const BUILTIN: &str = "Lnet/starlark/java/annot/StarlarkBuiltin;";
pub(crate) const METHOD: &str = "Lnet/starlark/java/annot/StarlarkMethod;";

/// The interfaces whose behaviour a Starlark facet implements.
pub const STARLARK_INTERFACES: &[&str] = &[
    "net/starlark/java/eval/StarlarkValue",
    "net/starlark/java/eval/Structure",
    "net/starlark/java/eval/StarlarkIterable",
    "net/starlark/java/eval/Sequence",
    "net/starlark/java/eval/StarlarkIndexable",
    "net/starlark/java/eval/StarlarkMembershipTestable",
    "net/starlark/java/eval/StarlarkCallable",
];

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Builtin {
    pub name: String,
    pub doc: String,
    pub documented: bool,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AllowedType {
    pub ty: Ty,
    pub generic1: Ty,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Param {
    pub name: String,
    pub doc: String,
    /// The Starlark expression of `defaultValue`; `None` when the parameter is required.
    pub default: Option<String>,
    /// Empty when the Java parameter type is the only allowed type.
    pub allowed: Vec<AllowedType>,
    pub named: bool,
    pub positional: bool,
}

/// One `@StarlarkMethod` and the Java method it sits on.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct StarlarkMethod {
    pub name: String,
    pub doc: String,
    pub documented: bool,
    pub struct_field: bool,
    pub params: Vec<Param>,
    pub extra_positionals: Option<String>,
    pub extra_keywords: Option<String>,
    pub self_call: bool,
    pub allow_return_nones: bool,
    pub use_thread: bool,
    pub use_semantics: bool,
    /// `enableOnlyWithFlag` / `disableWithFlag`: methods behind a Starlark flag.
    pub gated: bool,
    pub java_name: String,
    pub descriptor: String,
    /// The class in the hierarchy that declares the annotated method.
    pub declared_in: String,
}

impl StarlarkMethod {
    /// A line for `--dump` and for lining methods up across versions.
    pub fn shape(&self) -> String {
        let params: Vec<String> = self
            .params
            .iter()
            .map(|p| {
                let allowed: Vec<String> = p
                    .allowed
                    .iter()
                    .map(|a| {
                        if a.generic1 == object_ty() {
                            signature::pretty(&a.ty)
                        } else {
                            format!(
                                "{}<{}>",
                                signature::pretty(&a.ty),
                                signature::pretty(&a.generic1)
                            )
                        }
                    })
                    .collect();
                format!(
                    "{}{}{}{}",
                    p.name,
                    if allowed.is_empty() {
                        String::new()
                    } else {
                        format!(": {}", allowed.join(" | "))
                    },
                    p.default
                        .as_ref()
                        .map(|d| format!(" = {d}"))
                        .unwrap_or_default(),
                    match (p.positional, p.named) {
                        (true, true) => "",
                        (true, false) => " [pos]",
                        (false, true) => " [named]",
                        (false, false) => " [none]",
                    }
                )
            })
            .collect();
        let mut flags = Vec::new();
        if self.struct_field {
            flags.push("structField");
        }
        if self.self_call {
            flags.push("selfCall");
        }
        if self.allow_return_nones {
            flags.push("allowReturnNones");
        }
        if self.use_thread {
            flags.push("useStarlarkThread");
        }
        if self.use_semantics {
            flags.push("useStarlarkSemantics");
        }
        if self.gated {
            flags.push("flag-gated");
        }
        let mut extras = Vec::new();
        if let Some(n) = &self.extra_positionals {
            extras.push(format!("*{n}"));
        }
        if let Some(n) = &self.extra_keywords {
            extras.push(format!("**{n}"));
        }
        let mut all = params;
        all.extend(extras);
        format!(
            "{}({}) -> {} {}{}",
            self.name,
            all.join(", "),
            self.descriptor,
            if flags.is_empty() {
                String::new()
            } else {
                format!("[{}] ", flags.join(", "))
            },
            self.java_name
        )
    }
}

pub fn object_ty() -> Ty {
    Ty::Class {
        name: "java/lang/Object".into(),
        args: Vec::new(),
    }
}

fn find<'a, 'b>(anns: &'b [Annotation<'a>], desc: &str) -> Option<&'b Annotation<'a>> {
    anns.iter().find(|a| a.type_descriptor.to_string() == desc)
}

fn element<'a, 'b>(a: &'b Annotation<'a>, name: &str) -> Option<&'b AnnotationElementValue<'a>> {
    a.elements.iter().find(|e| e.name == name).map(|e| &e.value)
}

fn string(a: &Annotation, name: &str, default: &str) -> String {
    match element(a, name) {
        Some(AnnotationElementValue::StringConstant(s)) => s.to_string(),
        _ => default.to_string(),
    }
}

fn boolean(a: &Annotation, name: &str, default: bool) -> bool {
    match element(a, name) {
        Some(AnnotationElementValue::BooleanConstant(v)) => *v != 0,
        _ => default,
    }
}

fn nested<'a, 'b>(a: &'b Annotation<'a>, name: &str) -> Vec<&'b Annotation<'a>> {
    match element(a, name) {
        Some(AnnotationElementValue::ArrayValue(items)) => items
            .iter()
            .filter_map(|i| match i {
                AnnotationElementValue::AnnotationValue(a) => Some(a),
                _ => None,
            })
            .collect(),
        Some(AnnotationElementValue::AnnotationValue(a)) => vec![a],
        _ => Vec::new(),
    }
}

fn class_literal(a: &Annotation, name: &str) -> Option<Ty> {
    match element(a, name) {
        Some(AnnotationElementValue::ClassLiteral { class_name }) => {
            signature::parse_type(class_name)
        }
        _ => None,
    }
}

fn param(a: &Annotation) -> Param {
    let default = string(a, "defaultValue", "");
    Param {
        name: string(a, "name", ""),
        doc: string(a, "doc", ""),
        default: (!default.is_empty()).then_some(default),
        allowed: nested(a, "allowedTypes")
            .into_iter()
            .filter_map(|t| {
                Some(AllowedType {
                    ty: class_literal(t, "type")?,
                    generic1: class_literal(t, "generic1").unwrap_or_else(object_ty),
                })
            })
            .collect(),
        named: boolean(a, "named", false),
        positional: boolean(a, "positional", true),
    }
}

/// The `@StarlarkBuiltin` among a class's annotations.
pub fn builtin_of(anns: &[Annotation]) -> Option<Builtin> {
    let a = find(anns, BUILTIN)?;
    Some(Builtin {
        name: string(a, "name", ""),
        doc: string(a, "doc", ""),
        documented: boolean(a, "documented", true),
    })
}

/// The `@StarlarkMethod` among a method's annotations, for the Java method
/// `java_name` with `descriptor`; `declared_in` is filled in by the caller.
pub fn method_of(anns: &[Annotation], java_name: &str, descriptor: &str) -> Option<StarlarkMethod> {
    let a = find(anns, METHOD)?;
    let extra = |key: &str| {
        nested(a, key)
            .first()
            .map(|p| string(p, "name", ""))
            .filter(|n| !n.is_empty())
    };
    Some(StarlarkMethod {
        name: string(a, "name", ""),
        doc: string(a, "doc", ""),
        documented: boolean(a, "documented", true),
        struct_field: boolean(a, "structField", false),
        params: nested(a, "parameters").into_iter().map(param).collect(),
        extra_positionals: extra("extraPositionals"),
        extra_keywords: extra("extraKeywords"),
        self_call: boolean(a, "selfCall", false),
        allow_return_nones: boolean(a, "allowReturnNones", false),
        use_thread: boolean(a, "useStarlarkThread", false),
        use_semantics: boolean(a, "useStarlarkSemantics", false),
        gated: !string(a, "enableOnlyWithFlag", "").is_empty()
            || !string(a, "disableWithFlag", "").is_empty(),
        java_name: java_name.to_string(),
        descriptor: descriptor.to_string(),
        declared_in: String::new(),
    })
}

/// A field's `@Option`: what `OptionsBase` subclasses declare their flags with.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct OptionAnno {
    pub name: String,
    pub abbrev: Option<char>,
    pub help: String,
    pub value_help: String,
    pub default_value: String,
    pub category: String,
    pub documentation_category: String,
    pub effect_tags: Vec<String>,
    pub metadata_tags: Vec<String>,
    /// The `converter` class in internal form; `None` for the default (`Converter`),
    /// which means Bazel picks one from the field's type.
    pub converter: Option<String>,
    pub allow_multiple: bool,
    pub expansion: Vec<String>,
    pub implicit_requirements: Vec<String>,
    pub deprecation_warning: String,
    pub old_name: String,
    pub old_name_warning: bool,
}

const OPTION: &str = "Lcom/google/devtools/common/options/Option;";
const CONVERTER_DEFAULT: &str = "Lcom/google/devtools/common/options/Converter;";

fn enum_names(a: &Annotation, name: &str) -> Vec<String> {
    let one = |v: &AnnotationElementValue| match v {
        AnnotationElementValue::EnumConstant { const_name, .. } => Some(const_name.to_string()),
        _ => None,
    };
    match element(a, name) {
        Some(AnnotationElementValue::ArrayValue(items)) => items.iter().filter_map(one).collect(),
        Some(v) => one(v).into_iter().collect(),
        None => Vec::new(),
    }
}

fn string_array(a: &Annotation, name: &str) -> Vec<String> {
    let one = |v: &AnnotationElementValue| match v {
        AnnotationElementValue::StringConstant(s) => Some(s.to_string()),
        _ => None,
    };
    match element(a, name) {
        Some(AnnotationElementValue::ArrayValue(items)) => items.iter().filter_map(one).collect(),
        Some(v) => one(v).into_iter().collect(),
        None => Vec::new(),
    }
}

/// The `@Option` among a field's annotations, with the annotation's own
/// defaults for what the field leaves out.
pub fn option_of(anns: &[&Annotation]) -> Option<OptionAnno> {
    let a = anns
        .iter()
        .copied()
        .find(|a| a.type_descriptor.to_string() == OPTION)?;
    let abbrev = match element(a, "abbrev") {
        Some(AnnotationElementValue::CharConstant(c)) => u32::try_from(*c)
            .ok()
            .and_then(char::from_u32)
            .filter(|c| *c != '\0'),
        _ => None,
    };
    let converter = match element(a, "converter") {
        Some(AnnotationElementValue::ClassLiteral { class_name })
            if class_name != CONVERTER_DEFAULT =>
        {
            class_name
                .strip_prefix('L')
                .and_then(|c| c.strip_suffix(';'))
                .map(str::to_string)
        }
        _ => None,
    };
    let documentation_category = enum_names(a, "documentationCategory")
        .into_iter()
        .next()
        .unwrap_or_else(|| "UNCATEGORIZED".to_string());
    Some(OptionAnno {
        name: string(a, "name", ""),
        abbrev,
        help: string(a, "help", ""),
        value_help: string(a, "valueHelp", ""),
        default_value: string(a, "defaultValue", ""),
        category: string(a, "category", ""),
        documentation_category,
        effect_tags: enum_names(a, "effectTags"),
        metadata_tags: enum_names(a, "metadataTags"),
        converter,
        allow_multiple: boolean(a, "allowMultiple", false),
        expansion: string_array(a, "expansion"),
        implicit_requirements: string_array(a, "implicitRequirements"),
        deprecation_warning: string(a, "deprecationWarning", ""),
        old_name: string(a, "oldName", ""),
        old_name_warning: boolean(a, "oldNameWarning", true),
    })
}
