//! Resolving a spec against one version's classes: member selection,
//! signatures, Rust names, and the supertypes each class is a subtype of.

use crate::classfile::{ClassInfo, Classes, FieldInfo, MethodInfo};
use crate::ir::{
    Class, ImplMethod, Implementation, Member, MemberKind, ParamSig, Signature, SuperType, Target,
};
use crate::signature::{Arg, Ty};
use crate::spec::{ClassSpec, ImplementSpec, MemberSel, Spec, parse_member};
use crate::{jdk, names};
use anyhow::{Result, bail};
use std::collections::HashMap;
use std::rc::Rc;

/// Names every handle already has, which a member may not take without ` as <name>`.
const RESERVED: &[&str] = &[
    "clone",
    "eq",
    "ne",
    "hash",
    "fmt",
    "to_string",
    "drop",
    "from_raw",
    "into_raw",
    "borrowed",
    "upcast",
    "downcast",
    "is_a",
    "to_global",
    "null",
    "is_null",
];

pub fn lower(spec: &Spec, classes: &mut Classes, label: &str) -> Result<Vec<Class>> {
    let mut out: Vec<Class> = Vec::new();
    for cs in &spec.classes {
        let internal = cs.java.replace('.', "/");
        if out.iter().any(|c| c.internal == internal) {
            bail!("{}: listed twice in the spec; merge the entries", cs.java);
        }
        if jdk::lookup(&internal).is_some() {
            bail!(
                "{}: this standard Java type is provided by blazon-rt (blazon_rt::java) and is never generated; remove it from the spec",
                cs.java
            );
        }
        if jdk::is_jdk(&internal) {
            bail!(
                "{}: a JDK class that blazon-rt does not provide; add a handle to crates/blazon-rt/src/java and an entry to crates/blazon/src/jdk.rs",
                cs.java
            );
        }
        out.push(lower_class(cs, classes, label)?);
    }
    let all: Vec<String> = out.iter().map(|c| c.internal.clone()).collect();
    for c in &mut out {
        let info = classes.require(&c.internal, &c.java)?;
        c.supers = supertypes(&info, &all, classes)?;
    }
    inherit(&mut out, label)?;
    crate::facet::apply(spec, classes, label, &mut out)?;
    let all: Vec<String> = out.iter().map(|c| c.internal.clone()).collect();
    for c in &mut out {
        let info = classes.require(&c.internal, &c.java)?;
        c.supers = supertypes(&info, &all, classes)?;
        c.comparable = c
            .supers
            .iter()
            .any(|s| s.internal == "java/lang/Comparable")
            || implements(&info, "java/lang/Comparable", classes)?;
    }
    Ok(out)
}

/// The class, then its superclass chain, then its interfaces: where a member
/// can be inherited from, nearest first (the order `CallUtils` walks).
fn chain_of(classes: &mut Classes, info: &Rc<ClassInfo>) -> Result<Vec<Rc<ClassInfo>>> {
    let mut out: Vec<Rc<ClassInfo>> = vec![info.clone()];
    let mut i = 0;
    while i < out.len() {
        let parents: Vec<String> = out[i].supertypes().filter_map(class_name).collect();
        for p in parents {
            if out.iter().any(|c| c.name == p) {
                continue;
            }
            if let Some(c) = classes.get(&p)? {
                out.push(c);
            }
        }
        i += 1;
    }
    Ok(out)
}

/// What the spec selects on a bound class applies to every bound descendant:
/// ancestors first, each class takes the instance members of its bound
/// supertypes that it does not already have.
fn inherit(out: &mut [Class], label: &str) -> Result<()> {
    let mut order: Vec<usize> = (0..out.len()).collect();
    order.sort_by_key(|&i| out[i].supers.len());
    for ci in order {
        let supers = out[ci].supers.clone();
        for s in supers {
            let Some(ai) = out.iter().position(|c| c.internal == s.internal) else {
                continue;
            };
            if ai == ci {
                continue;
            }
            let map: HashMap<String, Ty> = out[ai]
                .type_params
                .iter()
                .cloned()
                .zip(s.args.iter().cloned())
                .collect();
            let ancestor = out[ai].internal.clone();
            let inherited: Vec<Member> = out[ai]
                .members
                .iter()
                .filter(|m| {
                    matches!(
                        m.kind,
                        MemberKind::Method(Target::Instance) | MemberKind::Field(Target::Instance)
                    )
                })
                .cloned()
                .collect();
            for mut m in inherited {
                let class = &out[ci];
                if class
                    .members
                    .iter()
                    .any(|x| x.java_name == m.java_name && x.descriptor == m.descriptor)
                {
                    continue;
                }
                if let Some(clash) = class.members.iter().find(|x| x.rust_name == m.rust_name) {
                    bail!(
                        "Bazel {label}: {} inherits {} from {}, which is named `{}` like its own {}; add ` as <name>` to one of them",
                        class.java,
                        m.selector,
                        ancestor.replace('/', "."),
                        names::bare(&m.rust_name),
                        clash.selector
                    );
                }
                for p in &mut m.sig.params {
                    p.ty = subst(&p.ty, &map);
                }
                m.sig.ret = subst(&m.sig.ret, &map);
                if m.declared_in.is_none() {
                    m.declared_in = Some(ancestor.clone());
                }
                out[ci].members.push(m);
            }
        }
    }
    Ok(())
}

fn implements(info: &ClassInfo, target: &str, classes: &mut Classes) -> Result<bool> {
    let mut stack: Vec<String> = info.supertypes().filter_map(class_name).collect();
    let mut seen = Vec::new();
    while let Some(n) = stack.pop() {
        if n == target {
            return Ok(true);
        }
        if seen.contains(&n) {
            continue;
        }
        seen.push(n.clone());
        if let Some(i) = classes.get(&n)? {
            stack.extend(i.supertypes().filter_map(class_name));
        }
    }
    Ok(false)
}

fn class_name(t: &Ty) -> Option<String> {
    match t {
        Ty::Class { name, .. } => Some(name.clone()),
        _ => None,
    }
}

pub(crate) fn lower_class(cs: &ClassSpec, classes: &mut Classes, label: &str) -> Result<Class> {
    let internal = cs.java.replace('.', "/");
    let info = classes.require(&internal, &format!("spec entry `{}`", cs.java))?;
    let type_params: Vec<String> = info.type_params.iter().map(|t| t.name.clone()).collect();
    if !cs.type_params.is_empty() && cs.type_params != type_params {
        bail!(
            "{}: the jar says type parameters {:?}, the spec says {:?}",
            cs.java,
            type_params,
            cs.type_params
        );
    }
    let chain = chain_of(classes, &info)?;
    let mut members: Vec<Member> = Vec::new();
    let mut explicit: Vec<bool> = Vec::new();
    for text in &cs.members {
        let sel = parse_member(text)?;
        let mut found = None;
        for ancestor in &chain {
            if let Some(mut m) = select(ancestor, &sel, &cs.java, label)? {
                if ancestor.name != info.name {
                    m.declared_in = Some(ancestor.name.clone());
                    for p in &mut m.sig.params {
                        p.ty = p.ty.erase_vars();
                    }
                    m.sig.ret = m.sig.ret.erase_vars();
                }
                found = Some(m);
                break;
            }
        }
        let Some(m) = found else {
            bail!(
                "Bazel {label}: {}: `{}` is not declared by the class or any of its supertypes",
                cs.java,
                sel.text
            );
        };
        explicit.push(sel.rust.is_some());
        members.push(m);
    }
    let mut class = Class {
        java: cs.java.clone(),
        internal,
        rust: cs.rust.clone().unwrap_or_else(|| names::class(&cs.java)),
        type_params,
        is_interface: info.is_interface,
        supers: Vec::new(),
        comparable: false,
        members,
        missing: Vec::new(),
        conditional: cs.conditional.clone(),
        starlark: None,
    };
    assign_names(&mut class, &explicit)?;
    Ok(class)
}

/// Fixes the Rust name of every member without ` as <name>`: the short
/// accessor name unless something else wants it, then the long one.
pub(crate) fn assign_names(class: &mut Class, explicit: &[bool]) -> Result<()> {
    let is_explicit = |i: usize| explicit.get(i).copied().unwrap_or(true);
    for (i, m) in class.members.iter().enumerate() {
        if is_explicit(i) && RESERVED.contains(&names::bare(&m.rust_name)) {
            bail!(
                "{}: {} is named `{}`, a name every handle already has; choose another",
                class.java,
                m.selector,
                names::bare(&m.rust_name)
            );
        }
    }
    let wanted: Vec<String> = class.members.iter().map(|m| m.rust_name.clone()).collect();
    let mut taken: HashMap<String, usize> = HashMap::new();
    for (i, name) in wanted.iter().enumerate() {
        if is_explicit(i) {
            if let Some(&other) = taken.get(name) {
                bail!(
                    "{}: {} and {} are both named `{}`",
                    class.java,
                    class.members[other].selector,
                    class.members[i].selector,
                    names::bare(name)
                );
            }
            taken.insert(name.clone(), i);
        }
    }
    for i in 0..class.members.len() {
        if is_explicit(i) {
            continue;
        }
        let contested = |n: &str| {
            taken.contains_key(n)
                || wanted
                    .iter()
                    .enumerate()
                    .any(|(j, w)| j != i && !is_explicit(j) && w == n)
        };
        let mut name = wanted[i].clone();
        if contested(&name) {
            let long = names::method_long(&class.members[i].java_name);
            if long != name && !contested(&long) {
                name = long;
            }
        }
        if let Some(&other) = taken.get(&name) {
            bail!(
                "{}: {} and {} both map to `{}`; add ` as <name>` to one of them",
                class.java,
                class.members[other].selector,
                class.members[i].selector,
                names::bare(&name)
            );
        }
        if RESERVED.contains(&names::bare(&name)) {
            bail!(
                "{}: {} maps to `{}`, a name every handle already has; add ` as <name>`",
                class.java,
                class.members[i].selector,
                names::bare(&name)
            );
        }
        taken.insert(name.clone(), i);
        class.members[i].rust_name = name;
    }
    Ok(())
}

pub(crate) fn select(
    info: &ClassInfo,
    sel: &MemberSel,
    java: &str,
    label: &str,
) -> Result<Option<Member>> {
    let methods: Vec<&MethodInfo> = info
        .methods
        .iter()
        .filter(|m| {
            !sel.field_only && m.name == sel.name && !m.access.synthetic && !m.access.bridge
        })
        .collect();
    let fields: Vec<&FieldInfo> = info.fields.iter().filter(|f| f.name == sel.name).collect();
    if let Some(params) = &sel.params {
        let want = format!("({params})");
        let hits: Vec<&&MethodInfo> = methods
            .iter()
            .filter(|m| m.descriptor.starts_with(&want))
            .collect();
        return match hits.len() {
            0 => Ok(None),
            1 => method_member(hits[0], sel).map(Some),
            _ => bail!(
                "{java}.{} matches several methods in Bazel {label}",
                sel.text
            ),
        };
    }
    match (methods.len(), fields.len()) {
        (0, 0) => Ok(None),
        (1, 0) => method_member(methods[0], sel).map(Some),
        (0, 1) => field_member(fields[0], sel).map(Some),
        (_, 0) => {
            let overloads: Vec<String> = methods
                .iter()
                .map(|m| format!("  {}({})", m.name, params_of(&m.descriptor)))
                .collect();
            bail!(
                "{java}.{} has {} overloads in Bazel {label}; pick one by its parameters:\n{}",
                sel.name,
                methods.len(),
                overloads.join("\n")
            )
        }
        _ => bail!(
            "{java}.{} names both a field and a method; write `{}(...)` for the method",
            sel.name,
            sel.name
        ),
    }
}

fn params_of(descriptor: &str) -> &str {
    descriptor
        .trim_start_matches('(')
        .split(')')
        .next()
        .unwrap_or("")
}

pub(crate) fn method_member(m: &MethodInfo, sel: &MemberSel) -> Result<Member> {
    let ctor = m.name == "<init>";
    let target = if m.access.is_static {
        Target::Static
    } else {
        Target::Instance
    };
    let kind = if ctor {
        MemberKind::Constructor
    } else {
        MemberKind::Method(target)
    };
    let returns_value = m.ret != Ty::Void;
    let rust_name = match (&sel.rust, ctor) {
        (Some(r), _) => r.clone(),
        (None, true) => "new".to_string(),
        (None, false) => names::method(&m.name, m.params.len(), returns_value),
    };
    Ok(Member {
        kind,
        java_name: m.name.clone(),
        rust_name,
        descriptor: m.descriptor.clone(),
        public: m.access.public,
        modifiers: m.access.modifiers(),
        sig: Signature {
            params: m
                .params
                .iter()
                .zip(&m.nullable_params)
                .enumerate()
                .map(|(i, (ty, n))| ParamSig {
                    ty: ty.clone(),
                    nullable: *n || sel.nullable_params.contains(&i),
                })
                .collect(),
            ret: m.ret.clone(),
            ret_nullable: m.nullable_ret || sel.nullable,
            type_params: m.type_params.clone(),
        },
        selector: sel.text.clone(),
        constant: None,
        doc: None,
        attribute: sel.attribute,
        declared_in: None,
    })
}

pub(crate) fn field_member(f: &FieldInfo, sel: &MemberSel) -> Result<Member> {
    let target = if f.access.is_static {
        Target::Static
    } else {
        Target::Instance
    };
    let constant = if f.access.is_static && f.access.is_final {
        f.constant.clone()
    } else {
        None
    };
    Ok(Member {
        kind: MemberKind::Field(target),
        java_name: f.name.clone(),
        rust_name: sel
            .rust
            .clone()
            .unwrap_or_else(|| names::ident(&names::snake(&f.name))),
        descriptor: f.descriptor.clone(),
        public: f.access.public,
        modifiers: f.access.modifiers(),
        sig: Signature {
            params: Vec::new(),
            ret: f.ty.clone(),
            ret_nullable: f.nullable || sel.nullable,
            type_params: Vec::new(),
        },
        selector: sel.text.clone(),
        constant,
        doc: None,
        attribute: sel.attribute,
        declared_in: None,
    })
}

/// Substitutes type variables.
pub(crate) fn subst(t: &Ty, map: &HashMap<String, Ty>) -> Ty {
    match t {
        Ty::Var(v) => map.get(v).cloned().unwrap_or_else(|| t.clone()),
        Ty::Class { name, args } => Ty::Class {
            name: name.clone(),
            args: args
                .iter()
                .map(|a| match a {
                    Arg::Exact(t) => Arg::Exact(subst(t, map)),
                    Arg::Extends(t) => Arg::Extends(subst(t, map)),
                    Arg::Super(t) => Arg::Super(subst(t, map)),
                    Arg::Any => Arg::Any,
                })
                .collect(),
        },
        Ty::Array(inner) => Ty::Array(Box::new(subst(inner, map))),
        other => other.clone(),
    }
}

/// Every bound or mapped supertype of the class, transitively, with type
/// arguments as seen from the class.
fn supertypes(info: &ClassInfo, bound: &[String], classes: &mut Classes) -> Result<Vec<SuperType>> {
    let mut out: Vec<SuperType> = Vec::new();
    let mut work: Vec<Ty> = info.supertypes().cloned().collect();
    while let Some(t) = work.pop() {
        let Ty::Class { name, args } = t else {
            continue;
        };
        if out.iter().any(|s| s.internal == name) {
            continue;
        }
        if jdk::lookup(&name).is_some() || bound.contains(&name) {
            out.push(SuperType {
                internal: name.clone(),
                args: args.iter().map(arg_bound).collect(),
            });
        }
        for m in jdk::mapped_supers(&name) {
            work.push(Ty::Class {
                name: (*m).to_string(),
                args: args.clone(),
            });
        }
        if let Some(sup) = classes.get(&name)? {
            let map: HashMap<String, Ty> = sup
                .type_params
                .iter()
                .zip(args.iter())
                .map(|(p, a)| (p.name.clone(), arg_bound(a)))
                .collect();
            for s in sup.supertypes() {
                work.push(subst(s, &map));
            }
        } else if name == "java/lang/Comparable" {
            out.retain(|s| s.internal != name);
            out.push(SuperType {
                internal: name,
                args: Vec::new(),
            });
        }
    }
    Ok(out)
}

/// A wildcard type argument as its bound; `?` and `? super X` as `Object`.
pub(crate) fn arg_bound(a: &Arg) -> Ty {
    match a {
        Arg::Exact(t) | Arg::Extends(t) => t.clone(),
        Arg::Any | Arg::Super(_) => crate::annot::object_ty(),
    }
}

/// Methods of `java.lang.Object` that an interface may redeclare; the
/// generated class inherits them.
const OBJECT_METHODS: &[(&str, &str)] = &[
    ("equals", "(Ljava/lang/Object;)Z"),
    ("hashCode", "()I"),
    ("toString", "()Ljava/lang/String;"),
];

/// The implementations the spec asks for, against one version's classes.
pub fn implementations(
    spec: &Spec,
    classes: &mut Classes,
    label: &str,
    bound: &[Class],
) -> Result<Vec<Implementation>> {
    let mut out: Vec<Implementation> = Vec::new();
    for is in &spec.implementations {
        let imp = implement(is, classes, label, bound)?;
        if out.iter().any(|o| o.internal == imp.internal) {
            bail!("{}: implemented twice in the spec", is.java);
        }
        if let Some(clash) = bound.iter().find(|c| c.rust == imp.trait_name) {
            bail!(
                "{}: the trait `{}` has the name of the bound class {}; set `rust = \"...\"`",
                is.java,
                imp.trait_name,
                clash.java
            );
        }
        out.push(imp);
    }
    Ok(out)
}

fn implement(
    is: &ImplementSpec,
    classes: &mut Classes,
    label: &str,
    bound: &[Class],
) -> Result<Implementation> {
    let internal = is.java.replace('.', "/");
    let who = format!("Bazel {label}: [[implement]] {}", is.java);
    if jdk::is_jdk(&internal) {
        bail!(
            "{who}: a JDK type cannot be implemented from Rust, because its handle lives in blazon-rt; wrap it in a type of the jar"
        );
    }
    let info = classes.require(&internal, &format!("[[implement]] `{}`", is.java))?;
    if !info.public {
        bail!("{who}: the type is not public, and the generated class lives in another package");
    }
    let (interface, super_internal) = if info.is_interface {
        let sup = match &is.extends {
            Some(e) => e.replace('.', "/"),
            None => "java/lang/Object".to_string(),
        };
        (Some(internal.clone()), sup)
    } else {
        if is.extends.is_some() {
            bail!(
                "{who}: it is a class, so it is what the implementation extends; `extends` is for an interface"
            );
        }
        (None, internal.clone())
    };
    let mut roots = vec![info.clone()];
    if super_internal != "java/lang/Object" {
        let sup = classes.require(&super_internal, &format!("{who}: `extends`"))?;
        if sup.is_interface {
            bail!(
                "{who}: {} is an interface, not a class to extend",
                super_internal.replace('/', ".")
            );
        }
        if !sup.public || sup.is_final {
            bail!(
                "{who}: {} must be a public, non-final class to extend",
                super_internal.replace('/', ".")
            );
        }
        let ctor = sup
            .methods
            .iter()
            .find(|m| m.name == "<init>" && m.descriptor == "()V");
        match ctor {
            Some(c) if c.access.public || c.access.protected => {}
            Some(_) => bail!(
                "{who}: {} has a no-argument constructor, but it is not public or protected",
                super_internal.replace('/', ".")
            ),
            None => bail!(
                "{who}: {} has no no-argument constructor, which the generated constructor calls; blazon cannot pass it arguments",
                super_internal.replace('/', ".")
            ),
        }
        roots.push(sup);
    }
    let mut chain: Vec<Rc<ClassInfo>> = Vec::new();
    for r in &roots {
        for c in chain_of(classes, r)? {
            if !chain.iter().any(|x| x.name == c.name) {
                chain.push(c);
            }
        }
    }
    let concrete: Vec<(&str, &str)> = chain
        .iter()
        .flat_map(|c| c.methods.iter())
        .filter(|m| !m.access.is_abstract && !m.access.is_static && !m.access.private)
        .map(|m| (m.name.as_str(), m.descriptor.as_str()))
        .collect();
    let mut abstracts: Vec<(&MethodInfo, &str)> = Vec::new();
    for c in &chain {
        for m in &c.methods {
            let key = (m.name.as_str(), m.descriptor.as_str());
            if !m.access.is_abstract
                || m.access.is_static
                || m.name == "<init>"
                || concrete.contains(&key)
                || OBJECT_METHODS.contains(&key)
                || abstracts
                    .iter()
                    .any(|(a, _)| a.name == m.name && a.descriptor == m.descriptor)
            {
                continue;
            }
            abstracts.push((m, &c.name));
        }
    }
    let mut chosen: Vec<(&MethodInfo, &str, MemberSel)> = Vec::new();
    if is.members.is_empty() {
        for (m, declared) in &abstracts {
            let sel = parse_member(&format!("{}({})", m.name, params_of(&m.descriptor)))?;
            chosen.push((m, declared, sel));
        }
    } else {
        for text in &is.members {
            let sel = parse_member(text)?;
            let hits: Vec<&(&MethodInfo, &str)> = abstracts
                .iter()
                .filter(|(m, _)| {
                    m.name == sel.name
                        && sel
                            .params
                            .as_ref()
                            .is_none_or(|p| m.descriptor.starts_with(&format!("({p})")))
                })
                .collect();
            match hits.as_slice() {
                [(m, declared)] => chosen.push((m, declared, sel)),
                [] => bail!(
                    "{who}: `{text}` is not an abstract method of the type; its abstract methods are: {}",
                    abstracts
                        .iter()
                        .map(|(m, _)| format!("{}({})", m.name, params_of(&m.descriptor)))
                        .collect::<Vec<_>>()
                        .join(", ")
                ),
                many => bail!(
                    "{who}: `{text}` has {} overloads; pick one by its parameters:\n{}",
                    many.len(),
                    many.iter()
                        .map(|(m, _)| format!("  {}({})", m.name, params_of(&m.descriptor)))
                        .collect::<Vec<_>>()
                        .join("\n")
                ),
            }
        }
    }
    for (m, declared, _) in &chosen {
        if !m.access.public && !m.access.protected {
            bail!(
                "{who}: {}.{} is package-private, so a class in another package cannot implement it",
                declared.replace('/', "."),
                m.name
            );
        }
    }
    let mut methods: Vec<ImplMethod> = Vec::new();
    for (m, declared, sel) in &chosen {
        let member = method_member(m, sel)?;
        methods.push(ImplMethod {
            java_name: m.name.clone(),
            rust_name: member.rust_name.clone(),
            descriptor: m.descriptor.clone(),
            sig: member.sig,
            selector: sel.text.clone(),
            declared_in: (*declared).to_string(),
        });
    }
    let explicit: Vec<bool> = chosen.iter().map(|(_, _, s)| s.rust.is_some()).collect();
    for i in 0..methods.len() {
        let name = methods[i].rust_name.clone();
        let dup = |n: &str| {
            methods
                .iter()
                .enumerate()
                .any(|(j, o)| j != i && o.rust_name == n)
        };
        if dup(&name) && !explicit[i] {
            let long = names::method_long(&methods[i].java_name);
            if !dup(&long) {
                methods[i].rust_name = long;
            }
        }
    }
    for (i, m) in methods.iter().enumerate() {
        if methods
            .iter()
            .enumerate()
            .any(|(j, o)| j != i && o.rust_name == m.rust_name)
        {
            bail!(
                "{who}: two methods are named `{}`; add ` as <name>` to one of them in `members`",
                names::bare(&m.rust_name)
            );
        }
    }
    let handle = bound
        .iter()
        .find(|c| c.internal == internal)
        .map(|c| c.rust.clone())
        .unwrap_or_else(|| names::class(&is.java));
    let trait_name = is.rust.clone().unwrap_or_else(|| format!("{handle}Impl"));
    let signature: Vec<String> = methods
        .iter()
        .map(|m| format!("{}{}", m.java_name, m.descriptor))
        .collect();
    let mut parts: Vec<&str> = vec![label, &internal, &super_internal];
    parts.extend(signature.iter().map(String::as_str));
    let short: String = handle
        .chars()
        .filter(|c| c.is_ascii_alphanumeric())
        .collect();
    let class = format!("blazon/impl/{short}${}", crate::classwriter::hash(&parts));
    let natives: Vec<crate::classwriter::Native> = methods
        .iter()
        .map(|m| crate::classwriter::Native {
            name: &m.java_name,
            descriptor: &m.descriptor,
        })
        .collect();
    let class_bytes =
        crate::classwriter::implementation(&class, &super_internal, interface.as_deref(), &natives);
    Ok(Implementation {
        java: is.java.clone(),
        internal,
        handle,
        trait_name,
        class,
        super_class: super_internal,
        methods,
        class_bytes,
        free_bytes: crate::classwriter::free(),
    })
}
