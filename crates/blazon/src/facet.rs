//! The Starlark side of resolution: which classes get a Starlark value type,
//! which methods it exposes, and which further classes must be bound for
//! those methods' signatures to be typed.
//!
//! A class with `@StarlarkBuiltin` or `@StarlarkMethod`s exposes exactly its
//! annotated methods, as `CallUtils` collects them from the class and its
//! ancestors. Any other class exposes the instance members its spec selects,
//! under their Java names. Classes named by those signatures are bound too,
//! opaque: no members, only the behaviour their Starlark interfaces give.

use crate::annot::{self, STARLARK_INTERFACES, StarlarkMethod};
use crate::classfile::{ClassInfo, Classes};
use crate::ir::{Class, Facet, FacetItem, FacetParam, Member, MemberKind};
use crate::lower;
use crate::signature::Ty;
use crate::spec::{ClassSpec, MemberSel, Spec};
use crate::{jdk, names};
use anyhow::{Result, anyhow, bail};
use std::collections::{BTreeSet, HashMap};
use std::rc::Rc;

/// What the generated Starlark types need from `net.starlark.java`: checked
/// per version, so a moved member is a generation error, not a runtime one.
pub const RUNTIME: &[(&str, &[&str])] = &[
    (
        "net.starlark.java.eval.Starlark",
        &[
            "NONE", "UNBOUND", "str", "repr", "len", "truth", "getattr", "dir", "call",
        ],
    ),
    (
        "net.starlark.java.eval.StarlarkThread",
        &["createTransient"],
    ),
    (
        "net.starlark.java.eval.Mutability",
        &["IMMUTABLE", "create"],
    ),
    (
        "net.starlark.java.eval.StarlarkSemantics",
        &["DEFAULT as default_semantics"],
    ),
    ("net.starlark.java.eval.StarlarkIndexable", &["getIndex"]),
    (
        "net.starlark.java.eval.StarlarkMembershipTestable",
        &["containsKey"],
    ),
    (
        "net.starlark.java.eval.StarlarkInt",
        &["of(J)", "toBigInteger"],
    ),
    ("net.starlark.java.eval.StarlarkList", &["empty"]),
    ("net.starlark.java.eval.Tuple", &["empty"]),
    ("net.starlark.java.eval.NoneType", &[]),
];

/// The Starlark surface of a class as the class files give it.
#[derive(Clone, Debug, Default)]
struct Info {
    builtin: Option<annot::Builtin>,
    methods: Vec<StarlarkMethod>,
    interfaces: BTreeSet<String>,
    ancestors: BTreeSet<String>,
}

struct Facets<'a> {
    classes: &'a mut Classes,
    label: &'a str,
    infos: HashMap<String, Rc<Info>>,
}

pub fn apply(spec: &Spec, classes: &mut Classes, label: &str, out: &mut Vec<Class>) -> Result<()> {
    let any = spec.starlark || spec.classes.iter().any(|c| c.starlark == Some(true));
    if !any {
        return Ok(());
    }
    let mut on: Vec<bool> = spec
        .classes
        .iter()
        .map(|c| c.starlark.unwrap_or(spec.starlark))
        .collect();
    on.resize(out.len(), false);
    let mut cx = Facets {
        classes,
        label,
        infos: HashMap::new(),
    };
    for (java, members) in RUNTIME {
        cx.ensure(out, &mut on, java, members)?;
    }
    let initial = on.clone();
    for (i, enabled) in initial.iter().enumerate() {
        if *enabled {
            cx.bind(out, i, false)?;
        }
    }
    let mut i = 0;
    while i < out.len() {
        if on[i] {
            cx.close(out, &mut on, i)?;
        }
        i += 1;
    }
    Ok(())
}

fn simple(internal: &str) -> String {
    internal
        .rsplit('/')
        .next()
        .unwrap_or(internal)
        .rsplit('$')
        .next()
        .unwrap_or(internal)
        .to_string()
}

fn class_refs(t: &Ty, out: &mut Vec<String>) {
    if let Ty::Class { name, .. } = t {
        if !out.contains(name) {
            out.push(name.clone());
        }
    }
}

/// Every class a member's signature names at the top level.
fn member_refs(m: &Member) -> Vec<String> {
    let mut out = Vec::new();
    for p in &m.sig.params {
        class_refs(&p.ty, &mut out);
    }
    class_refs(&m.sig.ret, &mut out);
    out
}

fn erase(t: &Ty) -> Ty {
    t.erase_vars()
}

impl Facets<'_> {
    /// Adds the class (no Starlark type of its own) or the missing members.
    fn ensure(
        &mut self,
        out: &mut Vec<Class>,
        on: &mut Vec<bool>,
        java: &str,
        members: &[&str],
    ) -> Result<()> {
        let spec = ClassSpec {
            java: java.to_string(),
            rust: None,
            members: members.iter().map(|m| m.to_string()).collect(),
            type_params: Vec::new(),
            starlark: Some(false),
        };
        let fresh = lower::lower_class(&spec, self.classes, self.label)?;
        match out.iter_mut().find(|c| c.java == java) {
            None => {
                out.push(fresh);
                on.push(false);
            }
            Some(existing) => {
                for m in fresh.members {
                    if !existing
                        .members
                        .iter()
                        .any(|e| e.java_name == m.java_name && e.descriptor == m.descriptor)
                    {
                        existing.members.push(m);
                    }
                }
                existing.missing.extend(fresh.missing);
            }
        }
        Ok(())
    }

    fn info(&mut self, internal: &str) -> Result<Rc<Info>> {
        if let Some(i) = self.infos.get(internal) {
            return Ok(i.clone());
        }
        let mut info = Info::default();
        let mut order: Vec<Rc<ClassInfo>> = Vec::new();
        let mut seen = BTreeSet::new();
        self.walk(internal, &mut order, &mut seen)?;
        for class in &order {
            if class.name != internal {
                info.ancestors.insert(class.name.clone());
            }
            if info.builtin.is_none() {
                info.builtin = class.builtin.clone();
            }
            for m in &class.methods {
                let Some(sm) = &m.starlark else { continue };
                if !info.methods.iter().any(|x| x.name == sm.name) {
                    let mut sm = sm.clone();
                    sm.declared_in = class.name.clone();
                    info.methods.push(sm);
                }
            }
        }
        // Supertypes outside the jar still count as ancestors by name.
        let mut named = Vec::new();
        for class in &order {
            for s in class.supertypes() {
                if let Ty::Class { name, .. } = s {
                    named.push(name.clone());
                }
            }
        }
        info.ancestors.extend(named);
        let mut more: Vec<String> = Vec::new();
        for a in &info.ancestors {
            more.extend(jdk::mapped_supers(a).iter().map(|s| (*s).to_string()));
        }
        info.ancestors.extend(more);
        for i in STARLARK_INTERFACES {
            if info.ancestors.contains(*i) || *i == internal {
                info.interfaces.insert((*i).to_string());
            }
        }
        info.methods.sort_by(|a, b| a.name.cmp(&b.name));
        let info = Rc::new(info);
        self.infos.insert(internal.to_string(), info.clone());
        Ok(info)
    }

    /// Depth-first: the class, its superclass, then its interfaces.
    fn walk(
        &mut self,
        name: &str,
        out: &mut Vec<Rc<ClassInfo>>,
        seen: &mut BTreeSet<String>,
    ) -> Result<()> {
        if !seen.insert(name.to_string()) {
            return Ok(());
        }
        let Some(class) = self.classes.get(name)? else {
            return Ok(());
        };
        out.push(class.clone());
        let parents: Vec<String> = class
            .supertypes()
            .filter_map(|t| match t {
                Ty::Class { name, .. } => Some(name.clone()),
                _ => None,
            })
            .collect();
        for p in parents {
            self.walk(&p, out, seen)?;
        }
        Ok(())
    }

    fn close(&mut self, out: &mut Vec<Class>, on: &mut Vec<bool>, i: usize) -> Result<()> {
        let owner = out[i].java.clone();
        let mut wanted: Vec<(String, String)> = Vec::new();
        for m in &out[i].members {
            for r in member_refs(m) {
                wanted.push((r, format!("{owner}.{}", m.selector)));
            }
        }
        if let Some(f) = &out[i].starlark {
            for (t, who) in &f.refs {
                let mut v = Vec::new();
                class_refs(t, &mut v);
                for r in v {
                    wanted.push((r, format!("{owner}.{who}")));
                }
            }
            for item in &f.items {
                for p in &item.params {
                    for t in &p.allowed {
                        let mut v = Vec::new();
                        class_refs(t, &mut v);
                        for r in v {
                            wanted.push((r, format!("{owner}.{} parameter {}", item.name, p.name)));
                        }
                    }
                }
            }
        }
        let label = self.label;
        for (internal, who) in wanted {
            if jdk::lookup(&internal).is_some() {
                continue;
            }
            if jdk::is_jdk(&internal) {
                bail!(
                    "Bazel {label}: {who} mentions {}, a JDK class that blazon-rt does not provide; add a handle to crates/blazon-rt/src/java and an entry to crates/blazon/src/jdk.rs",
                    internal.replace('/', ".")
                );
            }
            if out.iter().any(|c| c.internal == internal) {
                continue;
            }
            let java = internal.replace('/', ".");
            if !self.classes.contains(&internal) {
                bail!(
                    "Bazel {label}: {who} mentions {java}, which is not in the server jar; the Starlark binding cannot be typed"
                );
            }
            let rust = names::class_qualified(&internal);
            if let Some(other) = out.iter().find(|c| c.rust == rust) {
                bail!(
                    "Bazel {label}: {who} mentions {java}, which would be bound as `{rust}` like {}; list it in the spec with `rust = \"...\"`",
                    other.java
                );
            }
            let spec = ClassSpec {
                java,
                rust: Some(rust),
                members: Vec::new(),
                type_params: Vec::new(),
                starlark: Some(true),
            };
            let c = lower::lower_class(&spec, self.classes, label)?;
            out.push(c);
            on.push(true);
            let last = out.len() - 1;
            self.bind(out, last, true)?;
        }
        Ok(())
    }

    fn bind(&mut self, out: &mut [Class], i: usize, opaque: bool) -> Result<()> {
        let internal = out[i].internal.clone();
        let info = self.info(&internal)?;
        let annotated = info.builtin.is_some() || !info.methods.is_empty();
        let mut facet = Facet {
            type_name: info
                .builtin
                .as_ref()
                .map(|b| b.name.clone())
                .unwrap_or_else(|| simple(&internal)),
            doc: info
                .builtin
                .as_ref()
                .map(|b| b.doc.clone())
                .unwrap_or_default(),
            opaque,
            interfaces: info.interfaces.clone(),
            ancestors: info.ancestors.clone(),
            ..Facet::default()
        };
        facet.shapes.push((
            "@starlark builtin".into(),
            format!(
                "{}{}",
                facet.type_name,
                if info.builtin.is_some() {
                    ""
                } else {
                    " (no @StarlarkBuiltin)"
                }
            ),
        ));
        facet.shapes.push((
            "@starlark interfaces".into(),
            facet
                .interfaces
                .iter()
                .map(|s| simple(s))
                .collect::<Vec<_>>()
                .join(", "),
        ));
        if !opaque {
            if annotated {
                self.annotated_items(out, i, &info, &mut facet)?;
            } else {
                member_items(&mut out[i], &mut facet);
            }
        }
        out[i].starlark = Some(facet);
        Ok(())
    }

    /// A member for an annotated method, from the class that declares it.
    fn annotated_member(
        &mut self,
        class: &Class,
        sm: &StarlarkMethod,
        rust: String,
    ) -> Result<Member> {
        let declaring = self
            .classes
            .require(&sm.declared_in, &format!("{}.{}", class.java, sm.name))?;
        let m = declaring
            .methods
            .iter()
            .find(|m| m.name == sm.java_name && m.descriptor == sm.descriptor)
            .ok_or_else(|| {
                anyhow!(
                    "{}.{}: annotated method {} not found in its class file",
                    class.java,
                    sm.name,
                    sm.java_name
                )
            })?;
        let sel = MemberSel {
            name: sm.java_name.clone(),
            params: None,
            rust: Some(rust),
            nullable: sm.allow_return_nones,
            field_only: false,
            nullable_params: Vec::new(),
            attribute: false,
            text: format!("@starlark {}", sm.name),
        };
        let mut member = lower::method_member(m, &sel)?;
        if sm.declared_in != class.internal {
            for p in &mut member.sig.params {
                p.ty = erase(&p.ty);
            }
            member.sig.ret = erase(&member.sig.ret);
            member.sig.type_params.clear();
        }
        Ok(member)
    }

    fn annotated_items(
        &mut self,
        out: &mut [Class],
        i: usize,
        info: &Info,
        facet: &mut Facet,
    ) -> Result<()> {
        let label = self.label;
        for sm in &info.methods {
            facet
                .shapes
                .push((format!("@starlark {}", sm.name), sm.shape()));
            let skip = if sm.self_call {
                Some("selfCall; reached through StarlarkCallable")
            } else if sm.extra_positionals.is_some() || sm.extra_keywords.is_some() {
                Some("takes *args or **kwargs")
            } else if sm
                .params
                .iter()
                .any(|p| p.default.as_deref().is_some_and(|d| !supported_default(d)))
            {
                Some("a default value that is not a literal")
            } else {
                None
            };
            if let Some(why) = skip {
                facet.skipped.push((sm.name.clone(), why.to_string()));
                continue;
            }
            let existing = out[i]
                .members
                .iter()
                .position(|m| m.java_name == sm.java_name && m.descriptor == sm.descriptor);
            let idx = match existing {
                Some(idx) => idx,
                None => {
                    let rust = free_name(
                        &out[i],
                        &names::method(&sm.java_name, sm.params.len(), true),
                    );
                    let m = self.annotated_member(&out[i], sm, rust)?;
                    out[i].members.push(m);
                    out[i].members.len() - 1
                }
            };
            if out[i].members[idx].doc.is_none() {
                out[i].members[idx].doc = Some(names::plain_doc(&sm.doc)).filter(|d| !d.is_empty());
            }
            let m = &out[i].members[idx];
            let extra = usize::from(sm.use_thread) + usize::from(sm.use_semantics);
            if m.sig.params.len() != sm.params.len() + extra {
                bail!(
                    "Bazel {label}: {}.{} declares {} @Param(s) but {} takes {} Java parameter(s)",
                    out[i].java,
                    sm.name,
                    sm.params.len(),
                    sm.java_name,
                    m.sig.params.len()
                );
            }
            if !m.sig.params.iter().all(|p| bridgeable(&p.ty)) || !bridgeable(&m.sig.ret) {
                facet
                    .skipped
                    .push((sm.name.clone(), "array parameter or result".into()));
                continue;
            }
            facet.items.push(FacetItem {
                name: sm.name.clone(),
                doc: sm.doc.clone(),
                member: idx,
                attribute: sm.struct_field,
                params: sm
                    .params
                    .iter()
                    .map(|p| FacetParam {
                        name: p.name.clone(),
                        allowed: p.allowed.iter().map(|a| a.ty.clone()).collect(),
                        default: p.default.clone(),
                        named: p.named,
                        positional: p.positional,
                    })
                    .collect(),
                use_thread: sm.use_thread,
                use_semantics: sm.use_semantics,
            });
            for p in &sm.params {
                for a in &p.allowed {
                    if a.generic1 != annot::object_ty() {
                        facet.refs.push((
                            a.generic1.clone(),
                            format!("{} parameter {}", sm.name, p.name),
                        ));
                    }
                }
            }
        }
        Ok(())
    }
}

fn free_name(class: &Class, base: &str) -> String {
    let mut name = base.to_string();
    let mut n = 1;
    while class.members.iter().any(|m| m.rust_name == name) {
        n += 1;
        name = if n == 2 {
            format!("{base}_starlark")
        } else {
            format!("{base}_starlark{n}")
        };
    }
    name
}

const SIMPLE_DEFAULTS: &[&str] = &["None", "True", "False", "unbound", "[]", "()"];

pub(crate) fn supported_default(d: &str) -> bool {
    SIMPLE_DEFAULTS.contains(&d)
        || d.parse::<i64>().is_ok()
        || (d.len() >= 2
            && ((d.starts_with('"') && d.ends_with('"'))
                || (d.starts_with('\'') && d.ends_with('\'')))
            && !d[1..d.len() - 1].contains(['"', '\'', '\\']))
}

/// A type a Starlark argument or result can be bridged through.
fn bridgeable(t: &Ty) -> bool {
    match t {
        Ty::Prim(_) => true,
        Ty::Var(_) | Ty::Class { .. } | Ty::Void => true,
        Ty::Array(_) => false,
    }
}

/// A class without annotations exposes its selected instance members.
fn member_items(class: &mut Class, facet: &mut Facet) {
    for (idx, m) in class.members.iter().enumerate() {
        if !matches!(
            m.kind,
            MemberKind::Method(crate::ir::Target::Instance)
                | MemberKind::Field(crate::ir::Target::Instance)
        ) {
            continue;
        }
        if !m.sig.params.iter().all(|p| bridgeable(&p.ty)) || !bridgeable(&m.sig.ret) {
            facet
                .skipped
                .push((m.java_name.clone(), "array parameter or result".into()));
            continue;
        }
        if m.attribute && !m.sig.params.is_empty() {
            facet.skipped.push((
                m.java_name.clone(),
                "an attribute cannot take parameters".into(),
            ));
            continue;
        }
        facet.items.push(FacetItem {
            name: if m.attribute {
                names::bare(&m.rust_name).to_string()
            } else {
                m.java_name.clone()
            },
            doc: String::new(),
            member: idx,
            attribute: m.kind.is_field() || m.attribute,
            params: (0..m.sig.params.len())
                .map(|n| FacetParam {
                    name: format!("arg{n}"),
                    allowed: Vec::new(),
                    default: None,
                    named: false,
                    positional: true,
                })
                .collect(),
            use_thread: false,
            use_semantics: false,
        });
    }
}

/// The Starlark surface of `internal` as `--dump` prints it.
pub fn describe(classes: &mut Classes, internal: &str) -> Result<String> {
    use std::fmt::Write;
    let mut cx = Facets {
        classes,
        label: "dump",
        infos: HashMap::new(),
    };
    let info = cx.info(internal)?;
    let mut out = String::new();
    if info.builtin.is_none() && info.methods.is_empty() && info.interfaces.is_empty() {
        return Ok(out);
    }
    match &info.builtin {
        Some(b) => {
            let _ = writeln!(
                out,
                "  starlark builtin {:?}{}",
                b.name,
                if b.documented { "" } else { " (undocumented)" }
            );
        }
        None => {
            let _ = writeln!(out, "  starlark builtin none");
        }
    }
    let _ = writeln!(
        out,
        "  starlark interfaces {}",
        info.interfaces
            .iter()
            .map(|i| simple(i))
            .collect::<Vec<_>>()
            .join(", ")
    );
    for m in &info.methods {
        let _ = writeln!(
            out,
            "  starlark method {}  (declared in {})",
            m.shape(),
            m.declared_in
        );
    }
    Ok(out)
}
