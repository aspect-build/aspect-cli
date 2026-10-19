//! Printing one member: the doc, the signature, and the body, which is the
//! cached-ID call plus the conversions the types call for.

use super::types::{Boxed, ParamKind, Prim, Problem, Ref, RetKind, Scope};
use crate::classfile::Constant;
use crate::ir::{Class, Member, MemberKind, Target};
use crate::signature::{self, Ty};
use anyhow::{Result, bail};
use proc_macro2::{Ident, Span, TokenStream};
use quote::{format_ident, quote};

pub fn id(name: &str) -> Ident {
    match name.strip_prefix("r#") {
        Some(raw) => Ident::new_raw(raw, Span::call_site()),
        None => Ident::new(name, Span::call_site()),
    }
}

/// The module-level static that holds a class's looked-up `Class`.
pub fn class_static(c: &Class) -> Ident {
    format_ident!("CLASS_{}", c.rust.to_uppercase().replace(['$', '.'], "_"))
}

pub(super) fn lifetime(name: &str) -> TokenStream {
    let l = syn::Lifetime::new(&format!("'{name}"), Span::call_site());
    quote!(#l)
}

/// The Rust type of a reference as a result: the handle borrows for `lt`,
/// and its type arguments are `'static` spellings.
pub fn ref_tokens(r: &Ref, lt: &TokenStream) -> TokenStream {
    match r {
        Ref::Handle { rust, args } => {
            let name = id(rust);
            let args = args.iter().map(ref_arg);
            quote!(#name<#lt #(, #args)*>)
        }
        Ref::Var(v) => {
            let v = id(v);
            quote!(<#v as blazon_rt::Rebind<#lt>>::Out)
        }
        Ref::Wild(_) => quote!(Object<#lt>),
    }
}

/// A reference as a type argument or a bound: `'static` all the way down.
pub fn ref_arg(r: &Ref) -> TokenStream {
    match r {
        Ref::Handle { rust, args } => {
            let name = id(rust);
            let args = args.iter().map(ref_arg);
            quote!(#name<'static #(, #args)*>)
        }
        Ref::Var(v) => {
            let v = id(v);
            quote!(#v)
        }
        Ref::Wild(n) => {
            let w = format_ident!("W{n}");
            quote!(#w)
        }
    }
}

fn anon(r: &Ref) -> TokenStream {
    ref_arg(r)
}

struct ParamCode {
    /// `name: Type`
    sig: TokenStream,
    /// Statements that make the argument.
    prelude: TokenStream,
    /// The argument to a JNI call.
    jvalue: TokenStream,
    /// The argument as a `&JObject`, for reflection; `None` for a primitive.
    object: Option<TokenStream>,
    /// For a primitive, the statements that box it for reflection.
    boxing: Option<TokenStream>,
}

impl ParamCode {
    fn object(sig: TokenStream, prelude: TokenStream, object: TokenStream) -> ParamCode {
        ParamCode {
            sig,
            prelude,
            jvalue: quote!(JValue::Object(#object)),
            object: Some(object),
            boxing: None,
        }
    }
}

fn jvalue_prim(p: Prim, var: &Ident) -> TokenStream {
    match p {
        Prim::Boolean => quote!(JValue::Bool(#var as u8)),
        Prim::Byte => quote!(JValue::Byte(#var)),
        Prim::Char => quote!(JValue::Char(#var)),
        Prim::Short => quote!(JValue::Short(#var)),
        Prim::Int => quote!(JValue::Int(#var)),
        Prim::Long => quote!(JValue::Long(#var)),
        Prim::Float => quote!(JValue::Float(#var)),
        Prim::Double => quote!(JValue::Double(#var)),
    }
}

pub(super) fn rust_prim(p: Prim) -> TokenStream {
    match p {
        Prim::Boolean => quote!(bool),
        Prim::Byte => quote!(i8),
        Prim::Char => quote!(u16),
        Prim::Short => quote!(i16),
        Prim::Int => quote!(i32),
        Prim::Long => quote!(i64),
        Prim::Float => quote!(f32),
        Prim::Double => quote!(f64),
    }
}

fn cell_method(p: Prim) -> Ident {
    format_ident!(
        "{}",
        match p {
            Prim::Boolean => "boolean",
            Prim::Byte => "byte",
            Prim::Char => "char",
            Prim::Short => "short",
            Prim::Int => "int",
            Prim::Long => "long",
            Prim::Float => "float",
            Prim::Double => "double",
        }
    )
}

fn box_fn(p: Prim) -> Option<Ident> {
    match p {
        Prim::Boolean => Some(format_ident!("box_bool")),
        Prim::Int => Some(format_ident!("box_int")),
        Prim::Long => Some(format_ident!("box_long")),
        _ => None,
    }
}

fn param_code(kind: &ParamKind, nullable: bool, var: &Ident, env: &TokenStream) -> ParamCode {
    match kind {
        ParamKind::Prim(p) => {
            let t = rust_prim(*p);
            ParamCode {
                sig: quote!(#var: #t),
                prelude: quote!(),
                jvalue: jvalue_prim(*p, var),
                object: None,
                boxing: box_fn(*p).map(|f| quote!(let #var = blazon_rt::conv::#f(#env, #var)?;)),
            }
        }
        ParamKind::Str if nullable => ParamCode::object(
            quote!(#var: Option<&str>),
            quote!(let #var = #var.map(|s| blazon_rt::conv::jstring(#env, s)).transpose()?;),
            quote!(&blazon_rt::conv::or_null(&#var)),
        ),
        ParamKind::Str => ParamCode::object(
            quote!(#var: &str),
            quote!(let #var = blazon_rt::conv::jstring(#env, #var)?;),
            quote!(#var.as_ref()),
        ),
        ParamKind::Boxed(b) => {
            let (t, f) = match b {
                Boxed::Bool => (quote!(bool), format_ident!("box_bool")),
                Boxed::Int => (quote!(i32), format_ident!("box_int")),
                Boxed::Long => (quote!(i64), format_ident!("box_long")),
            };
            if nullable {
                ParamCode::object(
                    quote!(#var: Option<#t>),
                    quote!(let #var = #var.map(|v| blazon_rt::conv::#f(#env, v)).transpose()?;),
                    quote!(&blazon_rt::conv::or_null(&#var)),
                )
            } else {
                ParamCode::object(
                    quote!(#var: #t),
                    quote!(let #var = blazon_rt::conv::#f(#env, #var)?;),
                    quote!(#var.as_ref()),
                )
            }
        }
        ParamKind::Handle(r) => {
            let t = anon(r);
            if nullable {
                ParamCode::object(
                    quote!(#var: Option<&impl blazon_rt::IsA<#t>>),
                    quote!(let #var = blazon_rt::conv::borrowed_opt(#var);),
                    quote!(&#var),
                )
            } else {
                ParamCode::object(
                    quote!(#var: &impl blazon_rt::IsA<#t>),
                    quote!(let #var = #var.borrowed();),
                    quote!(&#var),
                )
            }
        }
        ParamKind::Optional(r) => {
            let t = anon(r);
            ParamCode::object(
                quote!(#var: Option<&impl blazon_rt::IsA<#t>>),
                quote!(let #var = blazon_rt::conv::optional_arg(#env, #var)?;),
                quote!(#var.as_ref()),
            )
        }
        ParamKind::Bytes if nullable => ParamCode::object(
            quote!(#var: Option<&[u8]>),
            quote!(let #var = #var.map(|b| blazon_rt::conv::byte_array(#env, b)).transpose()?;),
            quote!(&blazon_rt::conv::or_null(&#var)),
        ),
        ParamKind::Bytes => ParamCode::object(
            quote!(#var: &[u8]),
            quote!(let #var = blazon_rt::conv::byte_array(#env, #var)?;),
            quote!(#var.as_ref()),
        ),
        ParamKind::StrArray => ParamCode::object(
            quote!(#var: &[impl AsRef<str>]),
            quote!(let #var = blazon_rt::conv::string_array(#env, #var)?;),
            quote!(#var.as_ref()),
        ),
        ParamKind::ObjArray { element, class } => {
            let t = anon(element);
            ParamCode::object(
                quote!(#var: &[impl blazon_rt::IsA<#t>]),
                quote!(let #var = blazon_rt::conv::object_array::<#t>(#env, #class, #var)?;),
                quote!(#var.as_ref()),
            )
        }
    }
}

/// The Rust result type, with `lt` for the handles in it.
pub(super) fn ret_type(kind: &RetKind, nullable: bool, lt: &TokenStream) -> TokenStream {
    let base = match kind {
        RetKind::Void => quote!(()),
        RetKind::Prim(p) => rust_prim(*p),
        RetKind::Str => quote!(String),
        RetKind::Boxed(Boxed::Bool) => quote!(bool),
        RetKind::Boxed(Boxed::Int) => quote!(i32),
        RetKind::Boxed(Boxed::Long) => quote!(i64),
        RetKind::Handle(r) => ref_tokens(r, lt),
        RetKind::Optional(r) => {
            let t = ref_tokens(r, lt);
            return quote!(Option<#t>);
        }
        RetKind::Bytes => quote!(Vec<u8>),
        RetKind::StrArray => quote!(Vec<String>),
        RetKind::ObjArray(r) => {
            let t = ref_tokens(r, lt);
            quote!(Vec<#t>)
        }
    };
    match kind {
        RetKind::Str
        | RetKind::Boxed(_)
        | RetKind::Handle(_)
        | RetKind::Bytes
        | RetKind::StrArray
        | RetKind::ObjArray(_)
            if nullable =>
        {
            quote!(Option<#base>)
        }
        _ => base,
    }
}

/// `Ok(value)` for the object `o` a call returned.
pub(super) fn ret_convert(kind: &RetKind, nullable: bool, what: &str) -> TokenStream {
    match (kind, nullable) {
        (RetKind::Str, false) => quote!(blazon_rt::conv::string(o, #what)),
        (RetKind::Str, true) => quote!(blazon_rt::conv::string_opt(o)),
        (RetKind::Boxed(b), n) => {
            let f = match (b, n) {
                (Boxed::Bool, false) => "boxed_bool",
                (Boxed::Bool, true) => "boxed_bool_opt",
                (Boxed::Int, false) => "boxed_int",
                (Boxed::Int, true) => "boxed_int_opt",
                (Boxed::Long, false) => "boxed_long",
                (Boxed::Long, true) => "boxed_long_opt",
            };
            let f = format_ident!("{f}");
            if n {
                quote!(blazon_rt::conv::#f(o))
            } else {
                quote!(blazon_rt::conv::#f(o, #what))
            }
        }
        (RetKind::Handle(_), false) => quote!(blazon_rt::conv::wrap(o, #what)),
        (RetKind::Handle(_), true) => quote!(blazon_rt::conv::wrap_opt(o)),
        (RetKind::Optional(r), _) => {
            let t = ref_arg(r);
            quote!(blazon_rt::conv::optional::<#t>(o, #what))
        }
        (RetKind::Bytes, false) => quote!(blazon_rt::conv::bytes(o, #what)),
        (RetKind::Bytes, true) => quote!(blazon_rt::conv::bytes_opt(o)),
        (RetKind::StrArray, false) => quote!(blazon_rt::conv::strings(o, #what)),
        (RetKind::StrArray, true) => quote!(blazon_rt::conv::strings_opt(o)),
        (RetKind::ObjArray(r), n) => {
            let t = ref_arg(r);
            if n {
                quote!(blazon_rt::conv::objects_opt::<#t>(o))
            } else {
                quote!(blazon_rt::conv::objects::<#t>(o, #what))
            }
        }
        (RetKind::Void | RetKind::Prim(_), _) => quote!(Ok(())),
    }
}

/// The text of a member's Java declaration, for its doc comment.
pub fn declaration(m: &Member) -> String {
    let tp = if m.sig.type_params.is_empty() {
        String::new()
    } else {
        format!("<{}> ", m.sig.type_params.join(", "))
    };
    let params: Vec<String> = m
        .sig
        .params
        .iter()
        .map(|p| signature::pretty(&p.ty))
        .collect();
    match m.kind {
        MemberKind::Field(_) => format!(
            "{} {} {}",
            m.modifiers,
            signature::pretty(&m.sig.ret),
            m.java_name
        ),
        MemberKind::Constructor => format!("{} {tp}<init>({})", m.modifiers, params.join(", ")),
        MemberKind::Method(_) => format!(
            "{} {tp}{} {}({})",
            m.modifiers,
            signature::pretty(&m.sig.ret),
            m.java_name,
            params.join(", ")
        ),
    }
}

fn doc(m: &Member, extra: &str) -> TokenStream {
    let extra = match &m.doc {
        Some(d) if extra.is_empty() => d.as_str(),
        _ => extra,
    };
    let decl = format!(" `{}`", declaration(m));
    let mut out = quote!(#[doc = #decl]);
    if !m.public {
        out.extend(quote!(#[doc = ""] #[doc = " Not public; reached through reflection."]));
    }
    if !extra.is_empty() {
        let extra = format!(" {extra}");
        out.extend(quote!(#[doc = ""] #[doc = #extra]));
    }
    out
}

fn problem(c: &Class, m: &Member, label: &str, p: Problem) -> anyhow::Error {
    anyhow::anyhow!(
        "Bazel {label}: {}.{}: {p}{}",
        c.java,
        m.selector,
        match p {
            Problem::Unbound(_) =>
                "; add it to the spec (members = [] is enough) or set `starlark = true`",
            _ => "",
        }
    )
}

/// A constant's Rust type and value.
fn constant(c: &Constant, descriptor: &str) -> (TokenStream, TokenStream) {
    match c {
        Constant::Int(v) => match descriptor {
            "Z" => {
                let b = *v != 0;
                (quote!(bool), quote!(#b))
            }
            "B" => {
                let v = *v as i8;
                (quote!(i8), quote!(#v))
            }
            "C" => {
                let v = *v as u16;
                (quote!(u16), quote!(#v))
            }
            "S" => {
                let v = *v as i16;
                (quote!(i16), quote!(#v))
            }
            _ => {
                let lit = proc_macro2::Literal::i32_unsuffixed(*v);
                (quote!(i32), quote!(#lit))
            }
        },
        Constant::Long(v) => {
            let lit = proc_macro2::Literal::i64_unsuffixed(*v);
            (quote!(i64), quote!(#lit))
        }
        Constant::Float(v) => (quote!(f32), quote!(#v)),
        Constant::Double(v) => (quote!(f64), quote!(#v)),
        Constant::Str(s) => (quote!(&'static str), quote!(#s)),
    }
}

/// Everything printed for one member: its functions, or its constant.
pub fn member(c: &Class, m: &Member, class_scope: &Scope, label: &str) -> Result<TokenStream> {
    let instance = m.kind.target() == Target::Instance;
    let class_vars_in_scope = instance || m.kind == MemberKind::Constructor;
    let mut vars: Vec<String> = if class_vars_in_scope {
        class_scope.class_vars.to_vec()
    } else {
        Vec::new()
    };
    vars.extend(m.sig.type_params.iter().cloned());
    let scope = Scope {
        bound: class_scope.bound,
        class_vars: &vars,
    };
    let scope = &scope;
    let name = id(&m.rust_name);
    let what = format!(
        "{}.{}",
        c.java.rsplit('.').next().unwrap_or(&c.java),
        m.java_name
    );
    let (class, decl_static) = match (&m.declared_in, m.public) {
        (Some(d), false) => (
            format_ident!("DECL"),
            quote!(static DECL: blazon_rt::member::ClassRef = blazon_rt::member::ClassRef::new(#d);),
        ),
        _ => (class_static(c), quote!()),
    };
    if let Some(k) = &m.constant {
        let (ty, value) = constant(k, &m.descriptor);
        let cname = id(&m.java_name);
        let decl = format!(" `{}`", declaration(m));
        let docs = quote!(#[doc = #decl]);
        return Ok(quote!(#docs pub const #cname: #ty = #value;));
    }
    let kind = m.kind;
    let env_expr = if instance {
        quote!(&mut env)
    } else {
        quote!(env)
    };
    let mut wildcards = 0usize;
    let mut codes = Vec::new();
    for (i, p) in m.sig.params.iter().enumerate() {
        let var = format_ident!("p{i}");
        let mut pk = scope.param(&p.ty).map_err(|e| problem(c, m, label, e))?;
        match &mut pk {
            ParamKind::Handle(r) | ParamKind::Optional(r) => r.number_wildcards(&mut wildcards),
            ParamKind::ObjArray { element, .. } => element.number_wildcards(&mut wildcards),
            _ => {}
        }
        codes.push((param_code(&pk, p.nullable, &var, &env_expr), var));
    }
    let ret = if kind == MemberKind::Constructor {
        RetKind::Handle(Ref::Handle {
            rust: c.rust.clone(),
            args: c.type_params.iter().map(|v| Ref::Var(v.clone())).collect(),
        })
    } else {
        scope
            .result(&m.sig.ret)
            .map_err(|e| problem(c, m, label, e))?
    };
    let nullable = m.sig.ret_nullable;
    let lt = if instance {
        lifetime("a")
    } else {
        lifetime("l")
    };
    let ret_ty = ret_type(&ret, nullable, &lt);
    let sig_params: Vec<&TokenStream> = codes.iter().map(|(c, _)| &c.sig).collect();
    let preludes: Vec<&TokenStream> = codes.iter().map(|(c, _)| &c.prelude).collect();
    let jvalues: Vec<&TokenStream> = codes.iter().map(|(c, _)| &c.jvalue).collect();
    let (receiver, env_param) = if instance {
        (quote!(&self,), quote!())
    } else {
        (quote!(), quote!(env: &mut JNIEnv<'l>,))
    };
    let method_vars: Vec<Ident> = m.sig.type_params.iter().map(|v| id(v)).collect();
    let wild_vars: Vec<Ident> = (0..wildcards).map(|n| format_ident!("W{n}")).collect();
    let generics = if instance {
        if method_vars.is_empty() && wild_vars.is_empty() {
            quote!()
        } else {
            quote!(<#(#method_vars: blazon_rt::Rebind<#lt>,)* #(#wild_vars),*>)
        }
    } else {
        quote!(<'l #(, #method_vars: blazon_rt::Rebind<'l>)* #(, #wild_vars)*>)
    };
    let env_decl = if instance {
        quote!(let mut env = blazon_rt::vm::env()?;)
    } else {
        quote!()
    };
    let env_use = if instance {
        quote!(&mut env)
    } else {
        quote!(env)
    };
    let self_obj = quote!(&self.obj);
    let desc = &m.descriptor;
    let jname = &m.java_name;
    let docs = doc(m, "");

    let mut reflect_preludes: Vec<TokenStream> = Vec::new();
    let body = if m.public || kind == MemberKind::Constructor {
        match kind {
            MemberKind::Method(Target::Instance) => {
                let call = call_expr(
                    &ret,
                    quote!(M),
                    quote!(#env_use, #self_obj, &[#(#jvalues),*]),
                    &what,
                    nullable,
                );
                quote!(static M: blazon_rt::member::Method = blazon_rt::member::Method::new(&#class, #jname, #desc); #call)
            }
            MemberKind::Method(Target::Static) => {
                let call = call_expr(
                    &ret,
                    quote!(M),
                    quote!(#env_use, &[#(#jvalues),*]),
                    &what,
                    nullable,
                );
                quote!(static M: blazon_rt::member::StaticMethod = blazon_rt::member::StaticMethod::new(&#class, #jname, #desc); #call)
            }
            MemberKind::Constructor if m.public => {
                let conv = ret_convert(&ret, false, &what);
                quote!(
                    static C: blazon_rt::member::Constructor = blazon_rt::member::Constructor::new(&#class, #desc);
                    let o = C.new_object(#env_use, &[#(#jvalues),*])?;
                    #conv
                )
            }
            MemberKind::Constructor => {
                let (extra, body) =
                    reflective_constructor(c, m, &ret, &codes, &what, &class, &env_use, label)?;
                reflect_preludes = extra;
                body
            }
            MemberKind::Field(Target::Instance) => {
                let call = call_expr(
                    &ret,
                    quote!(F),
                    quote!(#env_use, #self_obj),
                    &what,
                    nullable,
                );
                quote!(static F: blazon_rt::member::Field = blazon_rt::member::Field::new(&#class, #jname, #desc); #call)
            }
            MemberKind::Field(Target::Static) => {
                let call = call_expr(&ret, quote!(F), quote!(#env_use), &what, nullable);
                quote!(static F: blazon_rt::member::StaticField = blazon_rt::member::StaticField::new(&#class, #jname, #desc); #call)
            }
        }
    } else {
        let (extra, body) = reflective(
            c, m, &ret, &codes, &what, nullable, &class, &env_use, instance, label,
        )?;
        reflect_preludes = extra;
        body
    };
    let preludes: Vec<&TokenStream> = if m.public {
        preludes
    } else {
        reflect_preludes.iter().collect()
    };
    let getter = quote! {
        #docs
        pub fn #name #generics (#receiver #env_param #(#sig_params),*) -> blazon_rt::Result<#ret_ty> {
            #decl_static
            #env_decl
            #(#preludes)*
            #body
        }
    };
    let setter = if matches!(kind, MemberKind::Field(Target::Instance)) && !m.public {
        field_setter(c, m, scope, label, &class, &decl_static)?
    } else {
        quote!()
    };
    Ok(quote!(#getter #setter))
}

/// The expression that makes the call through a cell `cell` and returns the result.
fn call_expr(
    ret: &RetKind,
    cell: TokenStream,
    args: TokenStream,
    what: &str,
    nullable: bool,
) -> TokenStream {
    match ret {
        RetKind::Void => quote!(#cell.void(#args)?; Ok(())),
        RetKind::Prim(p) => {
            let f = cell_method(*p);
            quote!(#cell.#f(#args))
        }
        other => {
            let conv = ret_convert(other, nullable, what);
            quote!(let o = #cell.object(#args)?; #conv)
        }
    }
}

/// A non-public member: the reflection call, and the statements that make
/// its arguments (primitives boxed).
#[allow(clippy::too_many_arguments)]
fn reflective(
    c: &Class,
    m: &Member,
    ret: &RetKind,
    codes: &[(ParamCode, Ident)],
    what: &str,
    nullable: bool,
    class: &Ident,
    env_use: &TokenStream,
    instance: bool,
    label: &str,
) -> Result<(Vec<TokenStream>, TokenStream)> {
    let jname = &m.java_name;
    let conv = reflected_result(ret, nullable, what, c, m)?;
    if let MemberKind::Field(t) = m.kind {
        let this = if t == Target::Instance {
            quote!(Some(&self.obj))
        } else {
            quote!(None)
        };
        return Ok((
            Vec::new(),
            quote!(
                static F: blazon_rt::reflect::ReflectField = blazon_rt::reflect::ReflectField::new(&#class, #jname);
                let o = F.get(#env_use, #this)?;
                #conv
            ),
        ));
    }
    let Some(erased) = signature::parse_method(&m.descriptor) else {
        bail!(
            "Bazel {label}: {}.{}: bad descriptor {}",
            c.java,
            m.selector,
            m.descriptor
        );
    };
    let mut preludes = Vec::new();
    let mut args = Vec::new();
    for (code, _) in codes {
        match (&code.object, &code.boxing) {
            (Some(object), _) => {
                preludes.push(code.prelude.clone());
                args.push(object.clone());
            }
            (None, Some(boxing)) => {
                preludes.push(boxing.clone());
                let var = format_ident!(
                    "{}",
                    code.sig.to_string().split(':').next().unwrap_or("p").trim()
                );
                args.push(quote!(#var.as_ref()));
            }
            (None, None) => bail!(
                "Bazel {label}: {}.{}: a non-public member taking this primitive would need boxing, which blazon does not generate",
                c.java,
                m.selector
            ),
        }
    }
    let receiver = if instance {
        quote!(Some(&self.obj))
    } else {
        quote!(None)
    };
    let descriptors: Vec<String> = erased.params.iter().map(descriptor_of).collect();
    Ok((
        preludes,
        quote!(
            static M: blazon_rt::reflect::ReflectMethod = blazon_rt::reflect::ReflectMethod::new(&#class, #jname, &[#(#descriptors),*]);
            let o = M.invoke(#env_use, #receiver, &[#(#args),*])?;
            #conv
        ),
    ))
}

/// A non-public constructor, reached through `getDeclaredConstructor`.
#[allow(clippy::too_many_arguments)]
fn reflective_constructor(
    c: &Class,
    m: &Member,
    ret: &RetKind,
    codes: &[(ParamCode, Ident)],
    what: &str,
    class: &Ident,
    env_use: &TokenStream,
    label: &str,
) -> Result<(Vec<TokenStream>, TokenStream)> {
    let Some(erased) = signature::parse_method(&m.descriptor) else {
        bail!(
            "Bazel {label}: {}.{}: bad descriptor {}",
            c.java,
            m.selector,
            m.descriptor
        );
    };
    let mut preludes = Vec::new();
    let mut args = Vec::new();
    for (code, _) in codes {
        match (&code.object, &code.boxing) {
            (Some(object), _) => {
                preludes.push(code.prelude.clone());
                args.push(object.clone());
            }
            (None, Some(boxing)) => {
                preludes.push(boxing.clone());
                let var = format_ident!(
                    "{}",
                    code.sig.to_string().split(':').next().unwrap_or("p").trim()
                );
                args.push(quote!(#var.as_ref()));
            }
            (None, None) => bail!(
                "Bazel {label}: {}.{}: a non-public constructor taking this primitive would need boxing, which blazon does not generate",
                c.java,
                m.selector
            ),
        }
    }
    let descriptors: Vec<String> = erased.params.iter().map(descriptor_of).collect();
    let conv = ret_convert(ret, false, what);
    Ok((
        preludes,
        quote!(
            static C: blazon_rt::reflect::ReflectConstructor = blazon_rt::reflect::ReflectConstructor::new(&#class, &[#(#descriptors),*]);
            let o = C.new_instance(#env_use, &[#(#args),*])?;
            #conv
        ),
    ))
}

fn reflected_result(
    ret: &RetKind,
    nullable: bool,
    what: &str,
    c: &Class,
    m: &Member,
) -> Result<TokenStream> {
    Ok(match ret {
        RetKind::Void => quote!(let _ = o; Ok(())),
        RetKind::Prim(Prim::Boolean) => quote!(blazon_rt::conv::boxed_bool(o, #what)),
        RetKind::Prim(Prim::Int) => quote!(blazon_rt::conv::boxed_int(o, #what)),
        RetKind::Prim(Prim::Long) => quote!(blazon_rt::conv::boxed_long(o, #what)),
        RetKind::Prim(p) => bail!(
            "{}.{}: a non-public member returning {p:?} would need unboxing, which blazon does not generate",
            c.java,
            m.selector
        ),
        other => ret_convert(other, nullable, what),
    })
}

fn descriptor_of(t: &Ty) -> String {
    match t {
        Ty::Prim(c) => c.to_string(),
        Ty::Void => "V".into(),
        Ty::Class { name, .. } => format!("L{name};"),
        Ty::Array(inner) => format!("[{}", descriptor_of(inner)),
        Ty::Var(_) => "Ljava/lang/Object;".into(),
    }
}

fn field_setter(
    c: &Class,
    m: &Member,
    scope: &Scope,
    label: &str,
    class: &Ident,
    decl_static: &TokenStream,
) -> Result<TokenStream> {
    let name = format_ident!(
        "set_{}",
        super::member::id(&m.rust_name)
            .to_string()
            .trim_start_matches("r#")
    );
    let var = format_ident!("value");
    let kind = scope
        .param(&m.sig.ret)
        .map_err(|e| problem(c, m, label, e))?;
    if let ParamKind::Prim(p) = &kind {
        if box_fn(*p).is_none() {
            bail!(
                "{}.{}: a non-public {p:?} field would need boxing to set, which blazon does not generate",
                c.java,
                m.selector
            );
        }
    }
    let code = param_code(&kind, m.sig.ret_nullable, &var, &quote!(&mut env));
    let sig = &code.sig;
    let (prelude, value) = match (&code.object, &code.boxing) {
        (Some(object), _) => (code.prelude.clone(), object.clone()),
        (None, Some(boxing)) => (boxing.clone(), quote!(#var.as_ref())),
        (None, None) => bail!(
            "{}.{}: a non-public field of this primitive type would need boxing to set, which blazon does not generate",
            c.java,
            m.selector
        ),
    };
    let jname = &m.java_name;
    let docs = format!(" Sets `{jname}` through reflection.");
    Ok(quote!(
        #[doc = #docs]
        pub fn #name(&self, #sig) -> blazon_rt::Result<()> {
            #decl_static
            let mut env = blazon_rt::vm::env()?;
            #prelude
            static F: blazon_rt::reflect::ReflectField = blazon_rt::reflect::ReflectField::new(&#class, #jname);
            F.set(&mut env, &self.obj, #value)
        }
    ))
}
