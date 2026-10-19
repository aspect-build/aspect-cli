//! Class files read into owned structures: what `blazon` needs of a class,
//! with generic signatures merged in and the annotations it cares about
//! (`@Nullable`, Starlark's) extracted.

use crate::annot::{self, Builtin, StarlarkMethod};
use crate::jar::Jar;
use crate::signature::{self, ClassTy, Ty, TypeParam};
use anyhow::{Context, Result, anyhow};
use cafebabe::attributes::{Annotation, AttributeData, AttributeInfo, TypeAnnotationTarget};
use cafebabe::{
    ClassAccessFlags, FieldAccessFlags, MethodAccessFlags, ParseOptions, parse_class_with_options,
};
use std::collections::HashMap;
use std::rc::Rc;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Access {
    pub public: bool,
    pub private: bool,
    pub protected: bool,
    pub is_static: bool,
    pub is_final: bool,
    pub is_abstract: bool,
    pub varargs: bool,
    pub synthetic: bool,
    pub bridge: bool,
}

impl Access {
    /// A short Java spelling, `public static final`.
    pub fn modifiers(&self) -> String {
        let mut out = vec![if self.public {
            "public"
        } else if self.private {
            "private"
        } else if self.protected {
            "protected"
        } else {
            "package-private"
        }];
        if self.is_static {
            out.push("static");
        }
        if self.is_final {
            out.push("final");
        }
        out.join(" ")
    }
}

#[derive(Clone, Debug, PartialEq)]
pub enum Constant {
    Int(i32),
    Long(i64),
    Float(f32),
    Double(f64),
    Str(String),
}

#[derive(Clone, Debug)]
pub struct FieldInfo {
    pub name: String,
    pub descriptor: String,
    /// With generics when the class file has a `Signature`.
    pub ty: Ty,
    pub access: Access,
    pub constant: Option<Constant>,
    pub nullable: bool,
    /// The field's `@Option`, for an options class.
    pub option: Option<crate::annot::OptionAnno>,
}

#[derive(Clone, Debug)]
pub struct MethodInfo {
    pub name: String,
    pub descriptor: String,
    pub type_params: Vec<String>,
    pub params: Vec<Ty>,
    pub ret: Ty,
    pub access: Access,
    pub nullable_ret: bool,
    pub nullable_params: Vec<bool>,
    pub starlark: Option<StarlarkMethod>,
}

#[derive(Clone, Debug)]
pub struct ClassInfo {
    pub name: String,
    pub public: bool,
    pub is_interface: bool,
    pub is_abstract: bool,
    pub is_final: bool,
    pub type_params: Vec<TypeParam>,
    pub super_class: Option<Ty>,
    pub interfaces: Vec<Ty>,
    pub fields: Vec<FieldInfo>,
    pub methods: Vec<MethodInfo>,
    pub builtin: Option<Builtin>,
}

impl ClassInfo {
    /// The direct supertypes with their type arguments.
    pub fn supertypes(&self) -> impl Iterator<Item = &Ty> {
        self.super_class.iter().chain(self.interfaces.iter())
    }
}

/// The classes of a jar, each read once.
pub struct Classes {
    jar: Jar,
    cache: HashMap<String, Option<Rc<ClassInfo>>>,
    superclasses: Option<Rc<HashMap<String, String>>>,
}

impl Classes {
    pub fn new(jar: Jar) -> Classes {
        Classes {
            jar,
            cache: HashMap::new(),
            superclasses: None,
        }
    }

    pub fn contains(&self, internal: &str) -> bool {
        self.jar.contains(internal)
    }

    /// The superclass of every class in the jar, read once.
    pub fn superclasses(&mut self) -> Result<Rc<HashMap<String, String>>> {
        if let Some(s) = &self.superclasses {
            return Ok(s.clone());
        }
        let s = Rc::new(self.jar.superclasses()?);
        self.superclasses = Some(s.clone());
        Ok(s)
    }

    /// The class, `None` when the jar does not have it.
    pub fn get(&mut self, internal: &str) -> Result<Option<Rc<ClassInfo>>> {
        if let Some(c) = self.cache.get(internal) {
            return Ok(c.clone());
        }
        let class = if self.jar.contains(internal) {
            let bytes = self.jar.class(internal)?;
            Some(Rc::new(read(&bytes).with_context(|| {
                format!("class {}", internal.replace('/', "."))
            })?))
        } else {
            None
        };
        self.cache.insert(internal.to_string(), class.clone());
        Ok(class)
    }

    /// The class, an error naming `what` when the jar does not have it.
    pub fn require(&mut self, internal: &str, what: &str) -> Result<Rc<ClassInfo>> {
        self.get(internal)?.ok_or_else(|| {
            anyhow!(
                "{what}: class {} is not in the server jar",
                internal.replace('/', ".")
            )
        })
    }
}

fn signature_attr(attrs: &[AttributeInfo]) -> Option<String> {
    attrs.iter().find_map(|a| match &a.data {
        AttributeData::Signature(s) => Some(s.to_string()),
        _ => None,
    })
}

fn annotations<'a, 'b>(attrs: &'b [AttributeInfo<'a>]) -> Vec<&'b Annotation<'a>> {
    let mut out = Vec::new();
    for a in attrs {
        match &a.data {
            AttributeData::RuntimeVisibleAnnotations(v)
            | AttributeData::RuntimeInvisibleAnnotations(v) => out.extend(v.iter()),
            _ => {}
        }
    }
    out
}

fn is_nullable(a: &Annotation) -> bool {
    let d = a.type_descriptor.to_string();
    let simple = d
        .trim_end_matches(';')
        .rsplit(['/', '$'])
        .next()
        .unwrap_or("");
    matches!(simple, "Nullable" | "CheckForNull")
}

fn nullable_return(attrs: &[AttributeInfo]) -> bool {
    if annotations(attrs).into_iter().any(is_nullable) {
        return true;
    }
    attrs.iter().any(|a| match &a.data {
        AttributeData::RuntimeVisibleTypeAnnotations(v)
        | AttributeData::RuntimeInvisibleTypeAnnotations(v) => v.iter().any(|t| {
            matches!(t.target_type, TypeAnnotationTarget::Empty)
                && t.target_path.is_empty()
                && is_nullable(&t.annotation)
        }),
        _ => false,
    })
}

fn nullable_params(attrs: &[AttributeInfo], count: usize) -> Vec<bool> {
    let mut out = vec![false; count];
    for a in attrs {
        match &a.data {
            AttributeData::RuntimeVisibleParameterAnnotations(v)
            | AttributeData::RuntimeInvisibleParameterAnnotations(v) => {
                // javac may omit leading synthetic parameters, so align from the end.
                let offset = count.saturating_sub(v.len());
                for (i, p) in v.iter().enumerate() {
                    if p.annotations.iter().any(is_nullable) && offset + i < count {
                        out[offset + i] = true;
                    }
                }
            }
            AttributeData::RuntimeVisibleTypeAnnotations(v)
            | AttributeData::RuntimeInvisibleTypeAnnotations(v) => {
                for t in v {
                    if let TypeAnnotationTarget::FormalParameter { index } = t.target_type {
                        if t.target_path.is_empty() && is_nullable(&t.annotation) {
                            if let Some(slot) = out.get_mut(index as usize) {
                                *slot = true;
                            }
                        }
                    }
                }
            }
            _ => {}
        }
    }
    out
}

/// The `ConstantValue` of a field. cafebabe does not export the type that
/// carries it, so it is read from its `Debug` form: `Integer(5)`, `String("x")`.
fn constant_of(attrs: &[AttributeInfo]) -> Option<Constant> {
    let text = attrs.iter().find_map(|a| match &a.data {
        AttributeData::ConstantValue(c) => Some(format!("{c:?}")),
        _ => None,
    })?;
    let (kind, rest) = text.split_once('(')?;
    let value = rest.strip_suffix(')')?;
    match kind {
        "Integer" => value.parse().ok().map(Constant::Int),
        "Long" => value.parse().ok().map(Constant::Long),
        "Float" => value.parse().ok().map(Constant::Float),
        "Double" => value.parse().ok().map(Constant::Double),
        "String" => unquote(value).map(Constant::Str),
        _ => None,
    }
}

/// Undoes the `Debug` escaping of a Rust string literal.
fn unquote(s: &str) -> Option<String> {
    let inner = s.strip_prefix('"')?.strip_suffix('"')?;
    let mut out = String::new();
    let mut chars = inner.chars();
    while let Some(c) = chars.next() {
        if c != '\\' {
            out.push(c);
            continue;
        }
        match chars.next()? {
            'n' => out.push('\n'),
            't' => out.push('\t'),
            'r' => out.push('\r'),
            '0' => out.push('\0'),
            '\\' => out.push('\\'),
            '"' => out.push('"'),
            '\'' => out.push('\''),
            'u' => {
                let rest: String = chars.by_ref().take_while(|c| *c != '}').collect();
                let code = u32::from_str_radix(rest.trim_start_matches('{'), 16).ok()?;
                out.push(char::from_u32(code)?);
            }
            _ => return None,
        }
    }
    Some(out)
}

fn method_access(f: MethodAccessFlags) -> Access {
    Access {
        public: f.contains(MethodAccessFlags::PUBLIC),
        private: f.contains(MethodAccessFlags::PRIVATE),
        protected: f.contains(MethodAccessFlags::PROTECTED),
        is_static: f.contains(MethodAccessFlags::STATIC),
        is_final: f.contains(MethodAccessFlags::FINAL),
        is_abstract: f.contains(MethodAccessFlags::ABSTRACT),
        varargs: f.contains(MethodAccessFlags::VARARGS),
        synthetic: f.contains(MethodAccessFlags::SYNTHETIC),
        bridge: f.contains(MethodAccessFlags::BRIDGE),
    }
}

fn field_access(f: FieldAccessFlags) -> Access {
    Access {
        public: f.contains(FieldAccessFlags::PUBLIC),
        private: f.contains(FieldAccessFlags::PRIVATE),
        protected: f.contains(FieldAccessFlags::PROTECTED),
        is_static: f.contains(FieldAccessFlags::STATIC),
        is_final: f.contains(FieldAccessFlags::FINAL),
        synthetic: f.contains(FieldAccessFlags::SYNTHETIC),
        ..Access::default()
    }
}

fn parse_descriptor_type(d: &str) -> Result<Ty> {
    signature::parse_type(d).ok_or_else(|| anyhow!("bad descriptor {d}"))
}

fn read(bytes: &[u8]) -> Result<ClassInfo> {
    let mut opts = ParseOptions::default();
    opts.parse_bytecode(false);
    let cf = parse_class_with_options(bytes, &opts).map_err(|e| anyhow!("{e}"))?;
    let name = cf.this_class.to_string();
    let class_sig: Option<ClassTy> =
        signature_attr(&cf.attributes).and_then(|s| signature::parse_class(&s));
    let object = |n: &str| Ty::Class {
        name: n.to_string(),
        args: Vec::new(),
    };
    let (type_params, super_class, interfaces) = match class_sig {
        Some(sig) => (
            sig.type_params,
            cf.super_class.as_ref().map(|_| sig.super_class),
            sig.interfaces,
        ),
        None => (
            Vec::new(),
            cf.super_class.as_ref().map(|s| object(&s.to_string())),
            cf.interfaces
                .iter()
                .map(|i| object(&i.to_string()))
                .collect(),
        ),
    };
    let mut fields = Vec::new();
    for f in &cf.fields {
        let descriptor = f.descriptor.to_string();
        let ty = match signature_attr(&f.attributes).and_then(|s| signature::parse_type(&s)) {
            Some(t) => t,
            None => parse_descriptor_type(&descriptor)?,
        };
        fields.push(FieldInfo {
            name: f.name.to_string(),
            descriptor,
            ty,
            access: field_access(f.access_flags),
            constant: constant_of(&f.attributes),
            nullable: nullable_return(&f.attributes),
            option: crate::annot::option_of(&annotations(&f.attributes)),
        });
    }
    let mut methods = Vec::new();
    for m in &cf.methods {
        let descriptor = m.descriptor.to_string();
        let erased = signature::parse_method(&descriptor)
            .ok_or_else(|| anyhow!("{}: bad descriptor {descriptor}", m.name))?;
        let generic = signature_attr(&m.attributes).and_then(|s| signature::parse_method(&s));
        let (type_params, params, ret) = match generic {
            Some(g) if g.params.len() == erased.params.len() => (g.type_params, g.params, g.ret),
            _ => (Vec::new(), erased.params, erased.ret),
        };
        let owned: Vec<&Annotation> = annotations(&m.attributes);
        let starlark = owned
            .iter()
            .find_map(|a| annot::method_of(std::slice::from_ref(*a), &m.name, &descriptor));
        let count = params.len();
        methods.push(MethodInfo {
            name: m.name.to_string(),
            descriptor,
            type_params,
            params,
            ret,
            access: method_access(m.access_flags),
            nullable_ret: nullable_return(&m.attributes),
            nullable_params: nullable_params(&m.attributes, count),
            starlark,
        });
    }
    let builtin = annotations(&cf.attributes)
        .into_iter()
        .find_map(|a| annot::builtin_of(std::slice::from_ref(a)));
    Ok(ClassInfo {
        name,
        public: cf.access_flags.contains(ClassAccessFlags::PUBLIC),
        is_interface: cf.access_flags.contains(ClassAccessFlags::INTERFACE),
        is_abstract: cf.access_flags.contains(ClassAccessFlags::ABSTRACT),
        is_final: cf.access_flags.contains(ClassAccessFlags::FINAL),
        type_params,
        super_class,
        interfaces,
        fields,
        methods,
        builtin,
    })
}
