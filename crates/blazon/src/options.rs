//! Bazel's command-line options as data: the `OptionsBase` subclasses in a
//! server jar, their `@Option` fields, and what each option's value is.
//!
//! The schema is read from the class files, so a version's flags are exactly
//! the ones that version declares. Which of them a consumer wants is a spec
//! decision (`[options]`), taken from Bazel's own effect tags.

use crate::annot::OptionAnno;
use crate::classfile::Classes;
use crate::signature::Ty;
use anyhow::Result;

const OPTIONS_BASE: &str = "com/google/devtools/common/options/OptionsBase";

/// One `@Option` field as the class file declares it.
#[derive(Clone, Debug)]
pub struct RawOption {
    /// The declaring class, internal form.
    pub class: String,
    pub field: String,
    /// The field's type, with generics when the class file has a `Signature`.
    pub ty: Ty,
    pub anno: OptionAnno,
}

/// Every class that extends `OptionsBase`, directly or transitively, with the
/// `@Option` fields it declares, sorted by class name.
pub fn discover(classes: &mut Classes) -> Result<Vec<(String, Vec<RawOption>)>> {
    let supers = classes.superclasses()?;
    let extends_base = |name: &str| {
        let mut cur = name;
        for _ in 0..64 {
            match supers.get(cur) {
                Some(s) if s == OPTIONS_BASE => return true,
                Some(s) => cur = s,
                None => return false,
            }
        }
        false
    };
    let mut names: Vec<String> = supers.keys().filter(|n| extends_base(n)).cloned().collect();
    names.sort();
    let mut out = Vec::new();
    for name in names {
        let info = classes.require(&name, "an options class")?;
        let options: Vec<RawOption> = info
            .fields
            .iter()
            .filter_map(|f| {
                Some(RawOption {
                    class: name.clone(),
                    field: f.name.clone(),
                    ty: f.ty.clone(),
                    anno: f.option.clone()?,
                })
            })
            .collect();
        out.push((name, options));
    }
    Ok(out)
}

/// What an option's value is, from its field type and converter.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum OptKind {
    Bool,
    Int,
    Long,
    Double,
    String,
    Label,
    Path,
    Duration,
    /// A flag that takes no value.
    Void,
    /// The enum's constants as declared; Bazel's converter matches them case-insensitively.
    Enum(Vec<String>),
    List(Box<OptKind>),
    /// A string-keyed dictionary of `key=value` assignments.
    Dict(Box<OptKind>),
    /// Anything else: accepted as a string and converted by this converter class
    /// (or, with the default converter, by the one for this field type).
    Opaque(String),
}

impl OptKind {
    /// The kind's name without its parameters, for counting.
    pub fn name(&self) -> &'static str {
        match self {
            OptKind::Bool => "Bool",
            OptKind::Int => "Int",
            OptKind::Long => "Long",
            OptKind::Double => "Double",
            OptKind::String => "String",
            OptKind::Label => "Label",
            OptKind::Path => "Path",
            OptKind::Duration => "Duration",
            OptKind::Void => "Void",
            OptKind::Enum(_) => "Enum",
            OptKind::List(_) => "List",
            OptKind::Dict(_) => "Dict",
            OptKind::Opaque(_) => "Opaque",
        }
    }

    /// A one-line spelling, for the shape report.
    pub fn describe(&self) -> String {
        match self {
            OptKind::Enum(v) => format!("Enum({})", v.join("|")),
            OptKind::List(k) => format!("List({})", k.describe()),
            OptKind::Dict(k) => format!("Dict({})", k.describe()),
            OptKind::Opaque(c) => format!("Opaque({c})"),
            other => other.name().to_string(),
        }
    }

    fn contains_opaque(&self) -> Option<&str> {
        match self {
            OptKind::Opaque(c) => Some(c),
            OptKind::List(k) | OptKind::Dict(k) => k.contains_opaque(),
            _ => None,
        }
    }
}

/// One option of the schema.
#[derive(Clone, Debug)]
pub struct OptionDef {
    pub name: String,
    pub abbrev: Option<char>,
    /// The declaring class, in Java binary form (`pkg.Outer$Inner`).
    pub class: String,
    pub field: String,
    pub kind: OptKind,
    pub default: String,
    pub help: String,
    pub value_help: String,
    pub category: String,
    pub effect_tags: Vec<String>,
    pub metadata_tags: Vec<String>,
    pub allows_multiple: bool,
    pub deprecated: bool,
    pub deprecation_warning: String,
    /// Sets other flags (`expansion`) rather than a value of its own.
    pub expansion: bool,
    pub expands_to: Vec<String>,
    pub implicit_requirements: Vec<String>,
    pub old_names: Vec<String>,
    pub fragment: bool,
}

impl OptionDef {
    /// One line that changes when anything a consumer relies on changes.
    pub fn shape(&self) -> String {
        format!(
            "{} default={:?} multiple={} class={} field={} effects={} meta={}{}",
            self.kind.describe(),
            self.default,
            self.allows_multiple,
            self.class,
            self.field,
            self.effect_tags.join("|"),
            self.metadata_tags.join("|"),
            if self.old_names.is_empty() {
                String::new()
            } else {
                format!(" old={}", self.old_names.join("|"))
            }
        )
    }
}

/// What was discovered and what the spec kept, for `--dump-options`.
#[derive(Clone, Debug, Default)]
pub struct Schema {
    /// The included options, sorted by name.
    pub options: Vec<OptionDef>,
    /// (class, discovered, included) for every option class.
    pub classes: Vec<(String, usize, usize)>,
    /// Every discovered option, for what the spec left out.
    pub discovered: usize,
}

const FRAGMENT_OPTIONS: &str = "com/google/devtools/build/lib/analysis/config/FragmentOptions";

fn simple_class(name: &str) -> &str {
    name.rsplit('/').next().unwrap_or(name)
}

fn dotted(internal: &str) -> String {
    internal.replace('/', ".")
}

/// The generic arguments of a class type, by simple class name.
fn parts(t: &Ty) -> Option<(&str, &[crate::signature::Arg])> {
    match t {
        Ty::Class { name, args } => Some((simple_class(name), args)),
        _ => None,
    }
}

fn arg_ty(a: &crate::signature::Arg) -> Option<&Ty> {
    match a {
        crate::signature::Arg::Exact(t) | crate::signature::Arg::Extends(t) => Some(t),
        _ => None,
    }
}

fn is_list(t: &Ty) -> Option<&Ty> {
    let (name, args) = parts(t)?;
    matches!(
        name,
        "List" | "ImmutableList" | "ImmutableSet" | "Set" | "Collection"
    )
    .then(|| args.first().and_then(arg_ty))
    .flatten()
}

fn class_of(t: &Ty) -> Option<&str> {
    match t {
        Ty::Class { name, .. } => Some(name),
        _ => None,
    }
}

/// How a converter class, by its simple name, reads a value, when blazon knows it.
fn known_converter(converter: &str) -> Option<OptKind> {
    let simple = simple_class(converter);
    Some(match simple {
        "Converters$BooleanConverter" => OptKind::Bool,
        "Converters$DurationConverter" => OptKind::Duration,
        "CoreOptionConverters$LabelConverter"
        | "CoreOptionConverters$EmptyToNullLabelConverter" => OptKind::Label,
        "OptionsUtils$PathFragmentConverter"
        | "OptionsUtils$EmptyToNullPathFragmentConverter"
        | "OptionsUtils$AbsolutePathFragmentConverter" => OptKind::Path,
        "Converters$CommaSeparatedOptionListConverter"
        | "Converters$CommaSeparatedNonEmptyOptionListConverter"
        | "Converters$CommaSeparatedOptionSetConverter"
        | "Converters$ColonSeparatedOptionListConverter" => {
            OptKind::List(Box::new(OptKind::String))
        }
        "CoreOptionConverters$LabelListConverter"
        | "CoreOptionConverters$LabelOrderedSetConverter" => {
            OptKind::List(Box::new(OptKind::Label))
        }
        "Converters$AssignmentConverter" | "Converters$EnvVarsConverter" => {
            OptKind::Dict(Box::new(OptKind::String))
        }
        "Converters$StringToStringListConverter" => {
            OptKind::Dict(Box::new(OptKind::List(Box::new(OptKind::String))))
        }
        _ => return None,
    })
}

/// The value kind of one element type (a field type, or a list's element),
/// `None` when the type alone does not say.
fn kind_of_type(t: &Ty, classes: &mut Classes) -> Result<Option<OptKind>> {
    Ok(match t {
        Ty::Prim('Z') => Some(OptKind::Bool),
        Ty::Prim('I') => Some(OptKind::Int),
        Ty::Prim('J') => Some(OptKind::Long),
        Ty::Prim('D') => Some(OptKind::Double),
        Ty::Class { name, args } => match simple_class(name) {
            "Boolean" => Some(OptKind::Bool),
            "Integer" => Some(OptKind::Int),
            "Long" => Some(OptKind::Long),
            "Double" => Some(OptKind::Double),
            "String" => Some(OptKind::String),
            "Label" => Some(OptKind::Label),
            "PathFragment" => Some(OptKind::Path),
            "Duration" => Some(OptKind::Duration),
            "Void" => Some(OptKind::Void),
            "Map$Entry" => dict_value(args, classes)?,
            _ => enum_constants(name, classes)?.map(OptKind::Enum),
        },
        _ => None,
    })
}

/// `Map.Entry<String, V>` as `Dict(V)`.
fn dict_value(args: &[crate::signature::Arg], classes: &mut Classes) -> Result<Option<OptKind>> {
    let (Some(k), Some(v)) = (args.first().and_then(arg_ty), args.get(1).and_then(arg_ty)) else {
        return Ok(None);
    };
    if class_of(k).map(simple_class) != Some("String") {
        return Ok(None);
    }
    if let Some((name, vargs)) = parts(v) {
        if let Some(inner) = is_list(v) {
            let _ = (name, vargs);
            return Ok(kind_of_type(inner, classes)?
                .map(|k| OptKind::Dict(Box::new(OptKind::List(Box::new(k))))));
        }
    }
    Ok(kind_of_type(v, classes)?.map(|k| OptKind::Dict(Box::new(k))))
}

/// The constants of an enum class, as declared; `None` when the class is not an enum.
fn enum_constants(internal: &str, classes: &mut Classes) -> Result<Option<Vec<String>>> {
    let Some(info) = classes.get(internal)? else {
        return Ok(None);
    };
    let is_enum =
        matches!(&info.super_class, Some(Ty::Class { name, .. }) if name == "java/lang/Enum");
    if !is_enum {
        return Ok(None);
    }
    let me = internal.to_string();
    Ok(Some(
        info.fields
            .iter()
            .filter(|f| {
                f.access.is_static
                    && f.access.is_final
                    && !f.access.synthetic
                    && matches!(&f.ty, Ty::Class { name, .. } if *name == me)
            })
            .map(|f| f.name.clone())
            .collect(),
    ))
}

/// What an option's value is. A converter blazon knows decides; otherwise the
/// field type does; a list of something is `List`, a repeated `Map.Entry` is
/// `Dict`; whatever is left is `Opaque`, named by its converter, or by the
/// field type when the converter is the default.
pub fn classify(raw: &RawOption, classes: &mut Classes) -> Result<OptKind> {
    let opaque = |raw: &RawOption| {
        OptKind::Opaque(match &raw.anno.converter {
            Some(c) => dotted(c),
            None => format!(
                "default converter for {}",
                crate::signature::pretty(&raw.ty)
            ),
        })
    };
    if let Some(conv) = &raw.anno.converter {
        if let Some(k) = known_converter(conv) {
            return Ok(k);
        }
    }
    let list_inner = is_list(&raw.ty);
    let element = list_inner.unwrap_or(&raw.ty);
    let Some(kind) = kind_of_type(element, classes)? else {
        return Ok(match list_inner {
            Some(_) => OptKind::List(Box::new(opaque(raw))),
            None => opaque(raw),
        });
    };
    // A converter that is not known may parse a primitive or a string in its
    // own way, so only a type whose reading is fixed by the type stays typed.
    let typed_by_converter_free = raw.anno.converter.is_none()
        || matches!(kind, OptKind::Enum(_) | OptKind::Dict(_) | OptKind::Void);
    if !typed_by_converter_free {
        return Ok(match list_inner {
            Some(_) => OptKind::List(Box::new(opaque(raw))),
            None => opaque(raw),
        });
    }
    Ok(match (list_inner, kind) {
        (Some(_), OptKind::Dict(v)) => OptKind::Dict(v),
        (Some(_), k) => OptKind::List(Box::new(k)),
        (None, k) => k,
    })
}

/// A class pattern: a dotted class name, or a prefix ending in `*`.
fn class_matches(pattern: &str, class: &str) -> bool {
    match pattern.strip_suffix('*') {
        Some(prefix) => class.starts_with(prefix),
        None => pattern == class,
    }
}

fn selected(o: &RawOption, fragment: bool, spec: &crate::spec::OptionsSpec) -> bool {
    let class = dotted(&o.class);
    let named = |v: &[String]| v.iter().any(|n| n == &o.anno.name);
    let in_class = |v: &[String]| v.iter().any(|c| class_matches(c, &class));
    if named(&spec.exclude) {
        return false;
    }
    if named(&spec.include) {
        return true;
    }
    if in_class(&spec.exclude_classes) {
        return false;
    }
    if o.anno
        .metadata_tags
        .iter()
        .any(|t| spec.exclude_metadata_tags.contains(t))
    {
        return false;
    }
    if in_class(&spec.include_classes) {
        return true;
    }
    if spec.fragments && fragment {
        return true;
    }
    let tags = &o.anno.effect_tags;
    let any_included = tags.iter().any(|t| spec.include_effect_tags.contains(t));
    let only_excluded =
        !tags.is_empty() && tags.iter().all(|t| spec.exclude_effect_tags.contains(t));
    any_included && !only_excluded
}

/// Discovers the jar's options and keeps what `spec` selects.
pub fn schema(
    classes: &mut Classes,
    spec: &crate::spec::OptionsSpec,
    label: &str,
) -> Result<Schema> {
    let supers = classes.superclasses()?;
    let is_fragment = |name: &str| {
        let mut cur = name;
        for _ in 0..64 {
            match supers.get(cur) {
                Some(s) if s == FRAGMENT_OPTIONS => return true,
                Some(s) => cur = s,
                None => return false,
            }
        }
        false
    };
    let mut out = Schema::default();
    for (class, raws) in discover(classes)? {
        let fragment = is_fragment(&class);
        let mut included = 0;
        for raw in &raws {
            out.discovered += 1;
            if !selected(raw, fragment, spec) {
                continue;
            }
            included += 1;
            let a = &raw.anno;
            let kind = classify(raw, classes)?;
            let documentation =
                if a.documentation_category == "UNCATEGORIZED" && !a.category.is_empty() {
                    a.category.clone()
                } else {
                    a.documentation_category.clone()
                };
            out.options.push(OptionDef {
                name: a.name.clone(),
                abbrev: a.abbrev,
                class: dotted(&raw.class),
                field: raw.field.clone(),
                kind,
                default: a.default_value.clone(),
                help: a.help.clone(),
                value_help: a.value_help.clone(),
                category: documentation,
                effect_tags: a.effect_tags.clone(),
                metadata_tags: a.metadata_tags.clone(),
                allows_multiple: a.allow_multiple,
                deprecated: !a.deprecation_warning.is_empty()
                    || a.metadata_tags.iter().any(|t| t == "DEPRECATED"),
                deprecation_warning: a.deprecation_warning.clone(),
                expansion: !a.expansion.is_empty(),
                expands_to: a.expansion.clone(),
                implicit_requirements: a.implicit_requirements.clone(),
                old_names: if a.old_name.is_empty() {
                    Vec::new()
                } else {
                    vec![a.old_name.clone()]
                },
                fragment,
            });
        }
        out.classes.push((dotted(&class), raws.len(), included));
    }
    out.options.sort_by(|a, b| a.name.cmp(&b.name));
    let mut clashes = Vec::new();
    let mut i = 0;
    while i < out.options.len() {
        let name = out.options[i].name.clone();
        let mut j = i;
        while j + 1 < out.options.len() && out.options[j + 1].name == name {
            j += 1;
        }
        if j > i {
            let classes: Vec<&str> = out.options[i..=j]
                .iter()
                .map(|o| o.class.as_str())
                .collect();
            clashes.push(format!("  {name}: {}", classes.join(", ")));
        }
        i = j + 1;
    }
    if !clashes.is_empty() {
        anyhow::bail!(
            "Bazel {label}: options declared by several classes; exclude all but one in [options]:\n{}",
            clashes.join("\n")
        );
    }
    Ok(out)
}

impl Schema {
    /// Counts and gaps, as `--dump-options` prints them.
    pub fn summary(&self) -> String {
        use std::collections::BTreeMap;
        use std::fmt::Write;
        let mut out = String::new();
        let _ = writeln!(
            out,
            "{} options discovered in {} classes, {} included",
            self.discovered,
            self.classes.len(),
            self.options.len()
        );
        let _ = writeln!(out, "\nper class (included of discovered):");
        for (class, total, included) in &self.classes {
            if *included > 0 {
                let _ = writeln!(out, "  {included:>3} / {total:<3} {class}");
            }
        }
        let mut kinds: BTreeMap<&str, usize> = BTreeMap::new();
        for o in &self.options {
            *kinds.entry(o.kind.name()).or_default() += 1;
        }
        let _ = writeln!(out, "\nper kind (included):");
        for (k, n) in &kinds {
            let _ = writeln!(out, "  {n:>4} {k}");
        }
        let _ = writeln!(out, "\nincluded options with Opaque values:");
        for o in &self.options {
            if let Some(c) = o.kind.contains_opaque() {
                let _ = writeln!(out, "  {} ({}): {c}", o.name, o.kind.describe());
            }
        }
        let _ = writeln!(out, "\nincluded options with no help text:");
        for o in &self.options {
            if o.help.is_empty() {
                let _ = writeln!(out, "  {} ({})", o.name, o.class);
            }
        }
        out
    }
}
