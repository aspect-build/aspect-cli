//! Printing the Rust side of an implemented Java type: the trait, the native
//! methods that forward to it, and the constructor that makes a Java object
//! owning a Rust value.

use super::member::{id, lifetime, ref_arg, ret_convert, ret_type, rust_prim};
use super::types::{Bound, ParamKind, Prim, Ref, RetKind, Scope};
use crate::ir::{ImplMethod, Implementation};
use crate::signature::Ty;
use proc_macro2::{Literal, TokenStream};
use quote::{format_ident, quote};
use std::collections::BTreeMap;

pub fn implementations(
    impls: &[Implementation],
    bound: &BTreeMap<String, Bound>,
    label: &str,
    problems: &mut Vec<String>,
) -> TokenStream {
    let Some(first) = impls.first() else {
        return quote!();
    };
    let free = Literal::byte_string(&first.free_bytes);
    let one: Vec<TokenStream> = impls
        .iter()
        .map(|i| implementation(i, bound, label, problems))
        .collect();
    quote!(
        static BLAZON_FREE_CLASS: &[u8] = #free;

        #(#one)*
    )
}

/// What a Java caller passes, as what a call into Java would return.
fn received(kind: &ParamKind) -> RetKind {
    match kind {
        ParamKind::Prim(p) => RetKind::Prim(*p),
        ParamKind::Str => RetKind::Str,
        ParamKind::Boxed(b) => RetKind::Boxed(*b),
        ParamKind::Handle(r) => RetKind::Handle(r.concrete()),
        ParamKind::Optional(r) => RetKind::Optional(r.concrete()),
        ParamKind::Bytes => RetKind::Bytes,
        ParamKind::StrArray => RetKind::StrArray,
        ParamKind::ObjArray { element, .. } => RetKind::ObjArray(element.concrete()),
    }
}

fn jni_prim(p: Prim) -> TokenStream {
    match p {
        Prim::Boolean => quote!(jni::sys::jboolean),
        Prim::Byte => quote!(jni::sys::jbyte),
        Prim::Char => quote!(jni::sys::jchar),
        Prim::Short => quote!(jni::sys::jshort),
        Prim::Int => quote!(jni::sys::jint),
        Prim::Long => quote!(jni::sys::jlong),
        Prim::Float => quote!(jni::sys::jfloat),
        Prim::Double => quote!(jni::sys::jdouble),
    }
}

fn jni_type(kind: &RetKind) -> TokenStream {
    match kind {
        RetKind::Void => quote!(()),
        RetKind::Prim(p) => jni_prim(*p),
        _ => quote!(jni::sys::jobject),
    }
}

/// The element class of a reference array, in internal form.
fn element_class(t: &Ty) -> String {
    match t {
        Ty::Array(inner) => match &**inner {
            Ty::Class { name, .. } => name.clone(),
            _ => "java/lang/Object".into(),
        },
        _ => "java/lang/Object".into(),
    }
}

/// The result of a native method from the Rust value `out`: an expression of
/// type `blazon_rt::Result<jni type>`.
fn returned(kind: &RetKind, nullable: bool, ret: &Ty) -> TokenStream {
    let out = quote!(out);
    match kind {
        RetKind::Void => quote!(Ok(())),
        RetKind::Prim(Prim::Boolean) => quote!(Ok(u8::from(#out))),
        RetKind::Prim(_) => quote!(Ok(#out)),
        RetKind::Str if nullable => quote!(blazon_rt::implement::string_opt(env, #out.as_deref())),
        RetKind::Str => quote!(blazon_rt::implement::string(env, &#out)),
        RetKind::Boxed(b) => {
            let name = match (b, nullable) {
                (super::types::Boxed::Bool, false) => "boxed_bool",
                (super::types::Boxed::Bool, true) => "boxed_bool_opt",
                (super::types::Boxed::Int, false) => "boxed_int",
                (super::types::Boxed::Int, true) => "boxed_int_opt",
                (super::types::Boxed::Long, false) => "boxed_long",
                (super::types::Boxed::Long, true) => "boxed_long_opt",
            };
            let f = format_ident!("{name}");
            quote!(blazon_rt::implement::#f(env, #out))
        }
        RetKind::Handle(_) if nullable => quote!(Ok(blazon_rt::implement::object_opt(#out))),
        RetKind::Handle(_) => quote!(Ok(blazon_rt::implement::object(#out))),
        RetKind::Optional(_) => quote!(blazon_rt::implement::optional(env, #out.as_ref())),
        RetKind::Bytes if nullable => {
            quote!(blazon_rt::implement::bytes_opt(env, #out.as_deref()))
        }
        RetKind::Bytes => quote!(blazon_rt::implement::bytes(env, &#out)),
        RetKind::StrArray if nullable => {
            quote!(blazon_rt::implement::strings_opt(env, #out.as_deref()))
        }
        RetKind::StrArray => quote!(blazon_rt::implement::strings(env, &#out)),
        RetKind::ObjArray(r) => {
            let element = ref_arg(r);
            let class = element_class(ret);
            if nullable {
                quote!(blazon_rt::implement::objects_opt::<#element, _>(env, #class, #out.as_deref()))
            } else {
                quote!(blazon_rt::implement::objects::<#element, _>(env, #class, &#out))
            }
        }
    }
}

fn implementation(
    i: &Implementation,
    bound: &BTreeMap<String, Bound>,
    label: &str,
    problems: &mut Vec<String>,
) -> TokenStream {
    let scope = Scope {
        bound,
        class_vars: &[],
    };
    let handle = id(&i.handle);
    let trait_name = id(&i.trait_name);
    let upper = i.trait_name.to_uppercase();
    let implementation = format_ident!("IMPL_{upper}");
    let class = &i.class;
    let internal = &i.internal;
    let bytes = Literal::byte_string(&i.class_bytes);
    let mut declarations = Vec::new();
    let mut natives = Vec::new();
    let mut trampolines = Vec::new();
    for (k, m) in i.methods.iter().enumerate() {
        match method(i, m, k, &scope, &trait_name, &implementation) {
            Ok((decl, native, tramp)) => {
                declarations.push(decl);
                natives.push(native);
                trampolines.push(tramp);
            }
            Err(e) => problems.push(format!(
                "Bazel {label}: [[implement]] {}.{}: {e}{}",
                i.java,
                m.selector,
                if e.contains("is not bound") {
                    "; add it to the spec (members = [] is enough)"
                } else {
                    ""
                }
            )),
        }
    }
    let docs = [
        format!(" Implements `{}` from Rust.", i.java),
        String::new(),
        format!(
            " [`{}::from_impl`] makes the Java object that calls it. An `Err` is thrown in Java",
            i.handle
        ),
        " as a `RuntimeException`, a panic as an `IllegalStateException`.".to_string(),
    ];
    let what = format!("{}::from_impl", i.handle);
    quote!(
        #(#[doc = #docs])*
        pub trait #trait_name: Send + Sync {
            #(#declarations)*
        }

        #(#trampolines)*

        static #implementation: blazon_rt::implement::Implementation =
            blazon_rt::implement::Implementation::new(&blazon_rt::implement::Def {
                class: #class,
                bytes: #bytes,
                free: BLAZON_FREE_CLASS,
                anchor: #internal,
                natives: &[#(#natives),*],
            });

        impl<'l> #handle<'l> {
            /// A Java object of the generated class that calls `value`; the value is
            /// dropped when the object is garbage collected.
            pub fn from_impl(env: &mut JNIEnv<'l>, value: Box<dyn #trait_name>) -> blazon_rt::Result<Self> {
                let o = #implementation.instantiate(env, value)?;
                blazon_rt::conv::wrap(o, #what)
            }
        }
    )
}

fn method(
    i: &Implementation,
    m: &ImplMethod,
    k: usize,
    scope: &Scope,
    trait_name: &proc_macro2::Ident,
    implementation: &proc_macro2::Ident,
) -> Result<(TokenStream, TokenStream, TokenStream), String> {
    let name = id(&m.rust_name);
    let tramp = format_ident!("__{}_{k}", i.handle);
    let a = lifetime("a");
    let s = lifetime("static");
    let what = format!("{}.{}", i.handle, m.java_name);
    let mut decl_params = Vec::new();
    let mut jni_params = Vec::new();
    let mut conversions = Vec::new();
    let mut args = Vec::new();
    for (n, p) in m.sig.params.iter().enumerate() {
        let var = format_ident!("arg{n}");
        let kind = scope.param(&p.ty).map_err(|e| e.to_string())?;
        let recv = received(&kind);
        let ty_a = ret_type(&recv, p.nullable, &a);
        let ty_s = ret_type(&recv, p.nullable, &s);
        let jty = jni_type(&recv);
        decl_params.push(quote!(#var: #ty_a));
        jni_params.push(quote!(#var: #jty));
        match &recv {
            RetKind::Prim(Prim::Boolean) => conversions.push(quote!(let #var = #var != 0;)),
            RetKind::Prim(p) => {
                let t = rust_prim(*p);
                conversions.push(quote!(let #var: #t = #var;));
            }
            other => {
                let conv = ret_convert(other, p.nullable, &what);
                conversions.push(quote!(
                    let #var: #ty_s = {
                        let o = blazon_rt::implement::local(env, #var)?;
                        #conv
                    }?;
                ));
            }
        }
        args.push(var);
    }
    let ret = scope.result(&m.sig.ret).map_err(|e| e.to_string())?;
    if let RetKind::Handle(Ref::Var(_)) = ret {
        return Err("a type variable result".into());
    }
    let ret_ty = ret_type(&ret, m.sig.ret_nullable, &a);
    let jret = jni_type(&ret);
    let arrow = if ret == RetKind::Void {
        quote!()
    } else {
        quote!(-> #jret)
    };
    let out = returned(&ret, m.sig.ret_nullable, &m.sig.ret);
    let call = if ret == RetKind::Void {
        quote!(imp.#name(env, #(#args),*)?;)
    } else {
        quote!(let out = imp.#name(env, #(#args),*)?;)
    };
    let decl_doc = format!(
        " `{} {}({})`",
        crate::signature::pretty(&m.sig.ret),
        m.java_name,
        m.sig
            .params
            .iter()
            .map(|p| crate::signature::pretty(&p.ty))
            .collect::<Vec<_>>()
            .join(", ")
    );
    let declared = format!(" Declared by `{}`.", m.declared_in.replace('/', "."));
    let decl = quote!(
        #[doc = #decl_doc]
        #[doc = ""]
        #[doc = #declared]
        fn #name<'a>(&self, env: &mut JNIEnv<'a>, #(#decl_params),*) -> blazon_rt::Result<#ret_ty>;
    );
    let jname = &m.java_name;
    let descriptor = &m.descriptor;
    let native = quote!(blazon_rt::implement::Native::new(
        #jname,
        #descriptor,
        #tramp as *const std::ffi::c_void
    ));
    let tramp_fn = quote!(
        unsafe extern "system" fn #tramp(env: *mut jni::sys::JNIEnv, this: jni::sys::jobject, #(#jni_params),*) #arrow {
            // SAFETY: a native method of the class `#implementation` defines, whose objects own a `Box<dyn #trait_name>`.
            unsafe {
                #implementation.dispatch::<Box<dyn #trait_name>, #jret>(env, this, #what, |env, imp| {
                    #(#conversions)*
                    #call
                    #out
                })
            }
        }
    );
    Ok((decl, native, tramp_fn))
}
