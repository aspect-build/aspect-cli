//! Printing the Starlark facet of each bound class: one value type holding a
//! global reference to the Java object, its `StarlarkValue` behaviour from
//! the Starlark interfaces the class implements, its methods and attributes
//! from `@StarlarkMethod`, and the per-version registry that wraps any Java
//! object as the most specific of these types.

use super::member::{id, ref_arg};
use super::types::{Bound, Boxed, ParamKind, Prim, Ref, RetKind, Scope};
use crate::ir::{Class, Facet, FacetItem, FacetParam};
use crate::signature::Ty;
use anyhow::{Result, anyhow};
use heck::ToSnakeCase;
use proc_macro2::{Ident, TokenStream};
use quote::{format_ident, quote};
use std::collections::BTreeMap;

/// The runtime classes and members the behaviours call, by their generated names.
struct Runtime {
    starlark: Ident,
    thread: Ident,
    mutability: Ident,
    semantics: Ident,
    indexable: Ident,
    membership: Ident,
    int: Ident,
    list: Ident,
    tuple: Ident,
    str: Ident,
    repr: Ident,
    len: Ident,
    truth: Ident,
    getattr: Ident,
    dir: Ident,
    call: Ident,
    none: Ident,
    unbound: Ident,
    create_transient: Ident,
    create: Ident,
    immutable: Ident,
    default_semantics: Ident,
    get_index: Ident,
    contains_key: Ident,
    int_of: Ident,
    int_to_big: Ident,
    list_empty: Ident,
    tuple_empty: Ident,
}

fn find(classes: &[Class], java: &str, member: &str, label: &str) -> Result<(Ident, Ident)> {
    let c = classes
        .iter()
        .find(|c| c.java == java)
        .ok_or_else(|| anyhow!("Bazel {label}: {java} is not bound"))?;
    let m = c.members.iter().find(|m| m.java_name == member).ok_or_else(|| {
        anyhow!("Bazel {label}: the Starlark runtime needs {java}.{member}, which this version does not have")
    })?;
    Ok((id(&c.rust), id(&m.rust_name)))
}

impl Runtime {
    fn new(classes: &[Class], label: &str) -> Result<Runtime> {
        const E: &str = "net.starlark.java.eval";
        let s = |n: &str, m: &str| find(classes, &format!("{E}.{n}"), m, label);
        Ok(Runtime {
            starlark: s("Starlark", "str")?.0,
            thread: s("StarlarkThread", "createTransient")?.0,
            mutability: s("Mutability", "IMMUTABLE")?.0,
            semantics: s("StarlarkSemantics", "DEFAULT")?.0,
            indexable: s("StarlarkIndexable", "getIndex")?.0,
            membership: s("StarlarkMembershipTestable", "containsKey")?.0,
            int: s("StarlarkInt", "of")?.0,
            list: s("StarlarkList", "empty")?.0,
            tuple: s("Tuple", "empty")?.0,
            str: s("Starlark", "str")?.1,
            repr: s("Starlark", "repr")?.1,
            len: s("Starlark", "len")?.1,
            truth: s("Starlark", "truth")?.1,
            getattr: s("Starlark", "getattr")?.1,
            dir: s("Starlark", "dir")?.1,
            call: s("Starlark", "call")?.1,
            none: s("Starlark", "NONE")?.1,
            unbound: s("Starlark", "UNBOUND")?.1,
            create_transient: s("StarlarkThread", "createTransient")?.1,
            create: s("Mutability", "create")?.1,
            immutable: s("Mutability", "IMMUTABLE")?.1,
            default_semantics: s("StarlarkSemantics", "DEFAULT")?.1,
            get_index: s("StarlarkIndexable", "getIndex")?.1,
            contains_key: s("StarlarkMembershipTestable", "containsKey")?.1,
            int_of: s("StarlarkInt", "of")?.1,
            int_to_big: s("StarlarkInt", "toBigInteger")?.1,
            list_empty: s("StarlarkList", "empty")?.1,
            tuple_empty: s("Tuple", "empty")?.1,
        })
    }
}

pub fn facets(
    label: &str,
    classes: &[Class],
    bound: &BTreeMap<String, Bound>,
    problems: &mut Vec<String>,
) -> Result<TokenStream> {
    if !classes.iter().any(|c| c.starlark.is_some()) {
        return Ok(quote!());
    }
    let rt = Runtime::new(classes, label)?;
    let Runtime {
        semantics,
        default_semantics,
        thread,
        mutability,
        create,
        create_transient,
        ..
    } = &rt;
    let header = quote!(
        use blazon_rt::Handle as _;
        use blazon_starlark::{JavaValue as _, ToJava as _};
        use starlark::values::ValueLike as _;
        use std::hash::Hasher as _;

        fn rt_semantics<'l>(env: &mut JNIEnv<'l>) -> blazon_rt::Result<#semantics<'l>> {
            #semantics::#default_semantics(env)
        }

    );
    let rt_thread = if needs_thread(classes) {
        quote!(
            fn rt_thread<'l>(env: &mut JNIEnv<'l>) -> blazon_rt::Result<#thread<'l>> {
                let mu = #mutability::#create(env, &[] as &[Object<'static>])?;
                let sem = rt_semantics(env)?;
                #thread::#create_transient(env, &mu, &sem)
            }
        )
    } else {
        quote!()
    };
    let header = quote!(#header #rt_thread);
    let mut out = vec![header];
    for c in classes {
        if let Some(f) = &c.starlark {
            out.push(class_facet(c, f, classes, bound, &rt, label, problems)?);
        }
    }
    out.push(registry(classes, &rt));
    Ok(quote!(#(#out)*))
}

/// Whether any behaviour or method needs a transient `StarlarkThread`.
fn needs_thread(classes: &[Class]) -> bool {
    classes
        .iter()
        .filter_map(|c| c.starlark.as_ref())
        .any(|f| has(f, "StarlarkCallable") || f.items.iter().any(|i| i.use_thread))
}

/// The Starlark value type of a class: `<Name>Value`, or `<Name>StarlarkValue`
/// when another bound class is already called `<Name>Value`.
fn value_name(c: &Class, classes: &[Class]) -> Ident {
    let base = c.rust.trim_start_matches("r#");
    let plain = format!("{base}Value");
    if classes.iter().any(|o| o.rust == plain) {
        format_ident!("{base}StarlarkValue")
    } else {
        format_ident!("{plain}")
    }
}

fn has(f: &Facet, iface: &str) -> bool {
    f.interfaces
        .iter()
        .any(|i| i.rsplit('/').next() == Some(iface))
}

fn class_facet(
    c: &Class,
    f: &Facet,
    classes: &[Class],
    bound: &BTreeMap<String, Bound>,
    rt: &Runtime,
    label: &str,
    problems: &mut Vec<String>,
) -> Result<TokenStream> {
    let v = value_name(c, classes);
    let r = id(&c.rust);
    let type_name = &f.type_name;
    let vars: Vec<Ident> = c.type_params.iter().map(|t| id(t)).collect();
    let generic_args = if vars.is_empty() {
        quote!()
    } else {
        quote!(, #(#vars),*)
    };
    let bounded = if vars.is_empty() {
        quote!()
    } else {
        quote!(, #(#vars: blazon_rt::Rebind<'a>),*)
    };
    let starlark_value = has(f, "StarlarkValue");
    let rt_s = &rt.starlark;
    let (str_, repr_) = (&rt.str, &rt.repr);
    let text = if starlark_value {
        quote!(
            let sem = rt_semantics(&mut env)?;
            #rt_s::#str_(&mut env, &obj, &sem)
        )
    } else {
        quote!(Ok(obj.to_string()))
    };
    let repr = if starlark_value {
        quote!(#rt_s::#repr_(&mut env, &obj))
    } else {
        quote!(Ok(obj.to_string()))
    };
    let behaviours = behaviours(f, &v, rt);
    let (methods_attr, methods_mod) = methods(c, f, &v, classes, bound, rt, label, problems)?;
    Ok(quote!(
        #[derive(starlark::values::ProvidesStaticType, starlark::values::NoSerialize, allocative::Allocative)]
        pub struct #v {
            #[allocative(skip)]
            pub obj: blazon_rt::Global<#r<'static>>,
        }

        starlark::starlark_simple_value!(#v);

        impl #v {
            pub fn new(obj: &jni::objects::JObject) -> blazon_rt::Result<Self> {
                let env = blazon_rt::vm::env()?;
                Ok(#v { obj: blazon_rt::Global::new(env.new_global_ref(obj)?) })
            }

            fn text(&self) -> blazon_rt::Result<String> {
                let mut env = blazon_rt::vm::env()?;
                let obj = self.obj.get(&mut env)?;
                #text
            }

            fn repr_text(&self) -> blazon_rt::Result<String> {
                let mut env = blazon_rt::vm::env()?;
                let obj = self.obj.get(&mut env)?;
                #repr
            }
        }

        impl<'a #bounded> #r<'a #generic_args> {
            /// This object as a Starlark value.
            pub fn to_value<'v>(&self, heap: starlark::values::Heap<'v>) -> blazon_rt::Result<starlark::values::Value<'v>> {
                let env = blazon_rt::vm::env()?;
                Ok(heap.alloc(#v { obj: blazon_rt::Global::new(env.new_global_ref(&self.obj)?) }))
            }
        }

        impl blazon_starlark::JavaValue for #v {
            fn java_object(&self) -> blazon_rt::Result<jni::objects::JObject<'static>> {
                let env = blazon_rt::vm::env()?;
                // SAFETY: a non-owning view of the global, for the call that copies it.
                Ok(env.new_local_ref(unsafe { jni::objects::JObject::from_raw(self.obj.raw()) })?)
            }
        }

        impl std::fmt::Display for #v {
            fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                match self.text() {
                    Ok(s) => f.write_str(&s),
                    Err(e) => write!(f, concat!("<", #type_name, ": {}>"), e),
                }
            }
        }

        impl std::fmt::Debug for #v {
            fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                f.write_str(stringify!(#v))
            }
        }

        #[starlark::values::starlark_value(type = #type_name)]
        impl<'v> starlark::values::StarlarkValue<'v> for #v {
            #methods_attr
            #behaviours
        }

        #methods_mod
    ))
}

fn behaviours(f: &Facet, v: &Ident, rt: &Runtime) -> TokenStream {
    let type_name = &f.type_name;
    let starlark_value = has(f, "StarlarkValue");
    let sequence = has(f, "Sequence");
    let is_collection = f.ancestors.contains("java/util/Collection");
    let is_map = f.ancestors.contains("java/util/Map");
    let is_list = f.ancestors.contains("java/util/List");
    let is_iterable = has(f, "StarlarkIterable")
        || sequence
        || is_collection
        || f.ancestors.contains("java/lang/Iterable");
    let Runtime {
        starlark,
        mutability,
        immutable,
        truth,
        len,
        getattr,
        dir,
        unbound,
        indexable,
        get_index,
        membership,
        contains_key,
        call,
        ..
    } = rt;
    let mut b = vec![quote!(
        fn collect_repr(&self, collector: &mut String) {
            match self.repr_text() {
                Ok(s) => collector.push_str(&s),
                Err(e) => collector.push_str(&format!(concat!("<", #type_name, ": {}>"), e)),
            }
        }
    )];
    if starlark_value {
        b.push(quote!(
            fn to_bool(&self) -> bool {
                (|| -> blazon_rt::Result<bool> {
                    let mut env = blazon_rt::vm::env()?;
                    let obj = self.obj.get(&mut env)?;
                    #starlark::#truth(&mut env, &obj)
                })()
                .unwrap_or(true)
            }
        ));
    } else if is_collection || is_map {
        b.push(quote!(
            fn to_bool(&self) -> bool {
                self.length().map(|n| n > 0).unwrap_or(true)
            }
        ));
    }
    if starlark_value && (sequence || is_collection || is_map) {
        b.push(quote!(
            fn length(&self) -> starlark::Result<i32> {
                (|| -> blazon_rt::Result<i32> {
                    let mut env = blazon_rt::vm::env()?;
                    let obj = self.obj.get(&mut env)?;
                    let n = #starlark::#len(&mut env, &obj)?;
                    if n < 0 {
                        return Err(blazon_rt::Error::msg(format!("{} has no length", #type_name)));
                    }
                    Ok(n)
                })()
                .map_err(blazon_starlark::error)
            }
        ));
    } else if is_collection {
        b.push(quote!(
            fn length(&self) -> starlark::Result<i32> {
                (|| -> blazon_rt::Result<i32> {
                    let mut env = blazon_rt::vm::env()?;
                    let c: Collection = self.obj.get(&mut env)?.downcast()?;
                    Ok(c.len()? as i32)
                })()
                .map_err(blazon_starlark::error)
            }
        ));
    } else if is_map {
        b.push(quote!(
            fn length(&self) -> starlark::Result<i32> {
                (|| -> blazon_rt::Result<i32> {
                    let mut env = blazon_rt::vm::env()?;
                    let m: Map = self.obj.get(&mut env)?.downcast()?;
                    Ok(m.len()? as i32)
                })()
                .map_err(blazon_starlark::error)
            }
        ));
    }
    if is_map && !has(f, "StarlarkIterable") {
        b.push(quote!(
            fn iterate_collect(
                &self,
                heap: starlark::values::Heap<'v>,
            ) -> starlark::Result<Vec<starlark::values::Value<'v>>> {
                blazon_rt::vm::scope(64, || {
                    let mut env = blazon_rt::vm::env()?;
                    let m: Map = self.obj.get(&mut env)?.downcast()?;
                    let mut out = Vec::new();
                    for k in m.keys()? {
                        out.push(blazon_starlark::wrap(k?.as_ref(), heap)?);
                    }
                    Ok(out)
                })
                .map_err(blazon_starlark::error)
            }
        ));
    } else if is_iterable {
        b.push(quote!(
            fn iterate_collect(
                &self,
                heap: starlark::values::Heap<'v>,
            ) -> starlark::Result<Vec<starlark::values::Value<'v>>> {
                blazon_rt::vm::scope(64, || {
                    let mut env = blazon_rt::vm::env()?;
                    let it: Iterable = self.obj.get(&mut env)?.downcast()?;
                    let mut out = Vec::new();
                    for item in it {
                        out.push(blazon_starlark::wrap(item?.as_ref(), heap)?);
                    }
                    Ok(out)
                })
                .map_err(blazon_starlark::error)
            }
        ));
    }
    if has(f, "StarlarkIndexable") || sequence {
        b.push(quote!(
            fn at(&self, index: starlark::values::Value<'v>, heap: starlark::values::Heap<'v>) -> starlark::Result<starlark::values::Value<'v>> {
                (|| -> blazon_rt::Result<starlark::values::Value<'v>> {
                    let mut env = blazon_rt::vm::env()?;
                    let sem = rt_semantics(&mut env)?;
                    let key = Object::of(index.to_java()?);
                    let ix: #indexable = self.obj.get(&mut env)?.downcast()?;
                    let r = ix.#get_index(&sem, &key)?;
                    blazon_starlark::wrap(r.as_ref(), heap)
                })()
                .map_err(blazon_starlark::error)
            }
        ));
    } else if is_list {
        b.push(quote!(
            fn at(
                &self,
                index: starlark::values::Value<'v>,
                heap: starlark::values::Heap<'v>,
            ) -> starlark::Result<starlark::values::Value<'v>> {
                (|| -> blazon_rt::Result<starlark::values::Value<'v>> {
                    let mut env = blazon_rt::vm::env()?;
                    let list: List = self.obj.get(&mut env)?.downcast()?;
                    let n = list.len()? as i64;
                    let i = index
                        .unpack_i32()
                        .map(i64::from)
                        .ok_or_else(|| blazon_rt::Error::msg("list index must be an int"))?;
                    let i = if i < 0 { i + n } else { i };
                    if i < 0 || i >= n {
                        return Err(blazon_rt::Error::msg(format!(
                            "index {i} out of range for length {n}"
                        )));
                    }
                    blazon_starlark::wrap(list.get(i as usize)?.as_ref(), heap)
                })()
                .map_err(blazon_starlark::error)
            }
        ));
    } else if is_map {
        b.push(quote!(
            fn at(
                &self,
                index: starlark::values::Value<'v>,
                heap: starlark::values::Heap<'v>,
            ) -> starlark::Result<starlark::values::Value<'v>> {
                (|| -> blazon_rt::Result<starlark::values::Value<'v>> {
                    let mut env = blazon_rt::vm::env()?;
                    let m: Map = self.obj.get(&mut env)?.downcast()?;
                    let key = Object::of(index.to_java()?);
                    match m.get(&key)? {
                        Some(v) => blazon_starlark::wrap(v.as_ref(), heap),
                        None => Err(blazon_rt::Error::msg(format!(
                            "key {} not found",
                            index.to_repr()
                        ))),
                    }
                })()
                .map_err(blazon_starlark::error)
            }
        ));
    }
    if has(f, "StarlarkMembershipTestable") || has(f, "StarlarkIndexable") || sequence {
        b.push(quote!(
            fn is_in(&self, other: starlark::values::Value<'v>) -> starlark::Result<bool> {
                (|| -> blazon_rt::Result<bool> {
                    let mut env = blazon_rt::vm::env()?;
                    let sem = rt_semantics(&mut env)?;
                    let key = Object::of(other.to_java()?);
                    let t: #membership = self.obj.get(&mut env)?.downcast()?;
                    t.#contains_key(&sem, &key)
                })()
                .map_err(blazon_starlark::error)
            }
        ));
    } else if is_collection {
        b.push(quote!(
            fn is_in(&self, other: starlark::values::Value<'v>) -> starlark::Result<bool> {
                (|| -> blazon_rt::Result<bool> {
                    let mut env = blazon_rt::vm::env()?;
                    let c: Collection = self.obj.get(&mut env)?.downcast()?;
                    c.contains(&Object::of(other.to_java()?))
                })()
                .map_err(blazon_starlark::error)
            }
        ));
    } else if is_map {
        b.push(quote!(
            fn is_in(&self, other: starlark::values::Value<'v>) -> starlark::Result<bool> {
                (|| -> blazon_rt::Result<bool> {
                    let mut env = blazon_rt::vm::env()?;
                    let m: Map = self.obj.get(&mut env)?.downcast()?;
                    m.contains_key(&Object::of(other.to_java()?))
                })()
                .map_err(blazon_starlark::error)
            }
        ));
    }
    if has(f, "Structure") {
        b.push(quote!(
            fn get_attr(&self, attribute: &str, heap: starlark::values::Heap<'v>) -> Option<starlark::values::Value<'v>> {
                (|| -> blazon_rt::Result<Option<starlark::values::Value<'v>>> {
                    let mut env = blazon_rt::vm::env()?;
                    let obj = self.obj.get(&mut env)?;
                    let sem = rt_semantics(&mut env)?;
                    let mu = #mutability::#immutable(&mut env)?;
                    let unbound = #starlark::#unbound(&mut env)?;
                    let r = #starlark::#getattr(&mut env, &mu, &sem, &obj, attribute, Some(&unbound))?;
                    if r == unbound {
                        return Ok(None);
                    }
                    Ok(Some(blazon_starlark::wrap(r.as_ref(), heap)?))
                })()
                .unwrap_or_else(|e| Some(blazon_starlark::failed(attribute, &e, heap)))
            }

            fn dir_attr(&self) -> Vec<String> {
                (|| -> blazon_rt::Result<Vec<String>> {
                    let mut env = blazon_rt::vm::env()?;
                    let obj = self.obj.get(&mut env)?;
                    let sem = rt_semantics(&mut env)?;
                    let mu = #mutability::#immutable(&mut env)?;
                    let names = #starlark::#dir(&mut env, &mu, &sem, &obj)?;
                    let names: Object = names.upcast();
                    let names: Iterable<JavaString<'static>> = names.downcast()?;
                    names.into_iter().map(|s| s?.to_rust()).collect()
                })()
                .unwrap_or_default()
            }
        ));
    }
    if has(f, "StarlarkCallable") {
        b.push(quote!(
            fn invoke(
                &self,
                _me: starlark::values::Value<'v>,
                args: &starlark::eval::Arguments<'v, '_>,
                eval: &mut starlark::eval::Evaluator<'v, '_, '_>,
            ) -> starlark::Result<starlark::values::Value<'v>> {
                let positional: Vec<starlark::values::Value<'v>> = args.positions(eval.heap())?.collect();
                let named = args.names_map()?;
                blazon_rt::vm::scope(64, || {
                    let mut env = blazon_rt::vm::env()?;
                    let obj = self.obj.get(&mut env)?;
                    let thread = rt_thread(&mut env)?;
                    let list: List<Object<'static>> = List::new(&mut env)?;
                    for p in &positional {
                        list.add(&Object::of(p.to_java()?))?;
                    }
                    let map: Map<JavaString<'static>, Object<'static>> = Map::new(&mut env)?;
                    for (k, val) in named.iter() {
                        let k = JavaString::new(&mut env, k.as_str())?;
                        map.put(&k, &Object::of(val.to_java()?))?;
                    }
                    let r = #starlark::#call(&mut env, &thread, &obj, &list, &map)?;
                    blazon_starlark::wrap(r.as_ref(), eval.heap())
                })
                .map_err(blazon_starlark::error)
            }
        ));
    }
    b.push(quote!(
        fn equals(&self, other: starlark::values::Value<'v>) -> starlark::Result<bool> {
            match other.downcast_ref::<#v>() {
                Some(o) => (|| -> blazon_rt::Result<bool> {
                    let mut env = blazon_rt::vm::env()?;
                    Ok(self.obj.get(&mut env)? == o.obj.get(&mut env)?)
                })()
                .map_err(blazon_starlark::error),
                None => Ok(false),
            }
        }

        fn write_hash(&self, hasher: &mut starlark::collections::StarlarkHasher) -> starlark::Result<()> {
            let h = (|| -> blazon_rt::Result<i32> {
                let mut env = blazon_rt::vm::env()?;
                blazon_rt::java::lang::hash_code(self.obj.get(&mut env)?.as_ref())
            })()
            .map_err(blazon_starlark::error)?;
            hasher.write_i32(h);
            Ok(())
        }
    ));
    quote!(#(#b)*)
}

/// The Rust type an argument is unpacked as: a precise union when every
/// allowed type is a scalar or a bound Starlark type, else any `Value`.
fn arg_type(p: &FacetParam, java: &Ty, classes: &[Class]) -> TokenStream {
    let candidates: Vec<Ty> = if p.allowed.is_empty() {
        vec![java.clone()]
    } else {
        p.allowed.clone()
    };
    let mut tys: Vec<TokenStream> = Vec::new();
    for t in &candidates {
        let one: TokenStream = match t {
            Ty::Prim('Z') => quote!(bool),
            Ty::Prim('I') => quote!(i32),
            Ty::Prim('J') => quote!(i64),
            Ty::Prim('D') | Ty::Prim('F') => quote!(f64),
            Ty::Class { name, .. } => match name.as_str() {
                "java/lang/String" => quote!(&'v str),
                "java/lang/Boolean" => quote!(bool),
                "java/lang/Integer" => quote!(i32),
                "java/lang/Long" | "net/starlark/java/eval/StarlarkInt" => quote!(i64),
                "net/starlark/java/eval/NoneType" => quote!(starlark::values::none::NoneType),
                other => match classes
                    .iter()
                    .find(|c| c.internal == other && c.starlark.is_some())
                {
                    Some(c) if other != "java/lang/Object" => {
                        let v = value_name(c, classes);
                        quote!(&'v #v)
                    }
                    _ => return quote!(starlark::values::Value<'v>),
                },
            },
            _ => return quote!(starlark::values::Value<'v>),
        };
        let text = one.to_string();
        if !tys.iter().any(|x| x.to_string() == text) {
            tys.push(one);
        }
    }
    let mut it = tys.into_iter().rev();
    let Some(mut acc) = it.next() else {
        return quote!(starlark::values::Value<'v>);
    };
    for t in it {
        acc = quote!(blazon_starlark::Either<#t, #acc>);
    }
    acc
}

/// An expression for the Java default of a parameter, as an `Object`.
fn default_object(d: &str, rt: &Runtime) -> TokenStream {
    let Runtime {
        starlark,
        none,
        unbound,
        list,
        list_empty,
        tuple,
        tuple_empty,
        ..
    } = rt;
    match d {
        "None" => quote!(Object::of(#starlark::#none(&mut env)?.into_raw())),
        "unbound" => quote!(#starlark::#unbound(&mut env)?),
        "[]" => quote!(Object::of(#list::#list_empty(&mut env)?.into_raw())),
        "()" => quote!(Object::of(#tuple::#tuple_empty(&mut env)?.into_raw())),
        "True" => quote!(Object::of(blazon_starlark::ToJava::to_java(&true)?)),
        "False" => quote!(Object::of(blazon_starlark::ToJava::to_java(&false)?)),
        n if n.parse::<i64>().is_ok() => {
            let n: i64 = n.parse().unwrap_or_default();
            quote!(Object::of(blazon_starlark::ToJava::to_java(&#n)?))
        }
        s => {
            let lit = &s[1..s.len() - 1];
            quote!(Object::of(blazon_starlark::ToJava::to_java(&#lit)?))
        }
    }
}

/// A Rust literal for a default of a scalar parameter, `None` when the
/// default is not a value of that type.
fn scalar_default(d: &str, kind: &ParamKind) -> Option<TokenStream> {
    match (kind, d) {
        (ParamKind::Prim(Prim::Boolean) | ParamKind::Boxed(Boxed::Bool), "True") => {
            Some(quote!(true))
        }
        (ParamKind::Prim(Prim::Boolean) | ParamKind::Boxed(Boxed::Bool), "False") => {
            Some(quote!(false))
        }
        (ParamKind::Prim(Prim::Int) | ParamKind::Boxed(Boxed::Int), n) => {
            n.parse::<i32>().ok().map(|n| quote!(#n))
        }
        (ParamKind::Prim(Prim::Long) | ParamKind::Boxed(Boxed::Long), n) => {
            n.parse::<i64>().ok().map(|n| quote!(#n))
        }
        (ParamKind::Prim(Prim::Byte | Prim::Short), n) => n.parse::<i32>().ok().map(|n| quote!(#n)),
        (ParamKind::Prim(Prim::Float | Prim::Double), n) => n
            .parse::<f64>()
            .ok()
            .map(|n| quote!(starlark::values::float::UnpackFloat(#n))),
        (ParamKind::Prim(Prim::Char), s)
            if s.len() >= 3 && (s.starts_with('"') || s.starts_with('\'')) =>
        {
            let lit = &s[1..s.len() - 1];
            (lit.chars().count() == 1).then(|| quote!(#lit))
        }
        (ParamKind::Str, s) if s.len() >= 2 && (s.starts_with('"') || s.starts_with('\'')) => {
            let lit = &s[1..s.len() - 1];
            Some(quote!(#lit))
        }
        _ => None,
    }
}

/// How a Starlark argument for a Java primitive parameter is unpacked: the
/// narrow integers as `int` (range-checked), `char` as a one-character
/// string, floating point as `int` or `float`.
fn prim_arg_type(p: Prim) -> TokenStream {
    match p {
        Prim::Boolean => quote!(bool),
        Prim::Byte | Prim::Short | Prim::Int => quote!(i32),
        Prim::Long => quote!(i64),
        Prim::Char => quote!(&'v str),
        Prim::Float | Prim::Double => quote!(starlark::values::float::UnpackFloat),
    }
}

fn prim_name(p: Prim) -> &'static str {
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
}

const RESERVED: &[&str] = &["self", "super", "crate", "Self", "this"];

#[allow(clippy::too_many_arguments)]
fn methods(
    c: &Class,
    f: &Facet,
    v: &Ident,
    classes: &[Class],
    bound: &BTreeMap<String, Bound>,
    rt: &Runtime,
    label: &str,
    problems: &mut Vec<String>,
) -> Result<(TokenStream, TokenStream)> {
    let fname = format_ident!(
        "{}_methods",
        c.rust.trim_start_matches("r#").to_snake_case()
    );
    let mut fns = Vec::new();
    let mut skipped: Vec<String> = f
        .skipped
        .iter()
        .map(|(n, why)| format!("`{n}`: {why}"))
        .collect();
    for item in &f.items {
        if RESERVED.contains(&item.name.as_str())
            || item
                .params
                .iter()
                .any(|p| RESERVED.contains(&p.name.as_str()))
        {
            skipped.push(format!(
                "`{}`: its name cannot be a Rust identifier",
                item.name
            ));
            continue;
        }
        match method(c, item, v, classes, bound, rt, label) {
            Ok(Ok(tokens)) => fns.push(tokens),
            Ok(Err(why)) => skipped.push(format!("`{}`: {why}", item.name)),
            Err(e) => problems.push(format!("{e}")),
        }
    }
    if fns.is_empty() && skipped.is_empty() {
        return Ok((quote!(), quote!()));
    }
    let notes = if skipped.is_empty() {
        quote!()
    } else {
        let mut lines = vec![" Not exposed to Starlark:".to_string()];
        lines.extend(skipped.iter().map(|s| format!(" - {s}")));
        quote!(#(#[doc = #lines])*)
    };
    let attr = if fns.is_empty() {
        quote!()
    } else {
        let name = fname.to_string();
        quote!(
            fn get_methods() -> Option<&'static starlark::environment::Methods> {
                static RES: starlark::environment::MethodsStatic = starlark::environment::MethodsStatic::new(#name, #fname);
                Some(RES.methods())
            }
        )
    };
    let module = if fns.is_empty() {
        quote!()
    } else {
        quote!(
            #notes
            #[starlark::starlark_module]
            fn #fname(builder: &mut starlark::environment::MethodsBuilder) {
                #(#fns)*
            }
        )
    };
    Ok((attr, module))
}

/// One method or attribute; `Ok(Err(reason))` when it cannot be exposed.
#[allow(clippy::too_many_arguments)]
fn method(
    c: &Class,
    item: &FacetItem,
    v: &Ident,
    classes: &[Class],
    bound: &BTreeMap<String, Bound>,
    rt: &Runtime,
    label: &str,
) -> Result<std::result::Result<TokenStream, String>> {
    let m = &c.members[item.member];
    let scope = Scope {
        bound,
        class_vars: &[],
    };
    let ident = id(&crate::names::ident(&item.name));
    let call_name = id(&m.rust_name);
    let ret = match scope.result(&m.sig.ret) {
        Ok(r) => r,
        Err(e) => return Ok(Err(e.to_string())),
    };
    let mut sig: Vec<TokenStream> = vec![quote!(this: &#v)];
    let mut prelude: Vec<TokenStream> = Vec::new();
    let mut call_args: Vec<TokenStream> = Vec::new();
    for (i, p) in item.params.iter().enumerate() {
        let java = &m.sig.params[i].ty;
        let kind = match scope.param(java) {
            Ok(k) => k,
            Err(e) => return Ok(Err(e.to_string())),
        };
        let rust_ty = match &kind {
            ParamKind::Prim(prim) => prim_arg_type(*prim),
            _ => arg_type(p, java, classes),
        };
        let name = id(&crate::names::ident(&p.name));
        let attr = match (p.positional, p.named) {
            (true, false) => quote!(#[starlark(require = pos)]),
            (false, true) => quote!(#[starlark(require = named)]),
            _ => quote!(),
        };
        let optional = p.default.is_some();
        let ty = if optional {
            quote!(Option<#rust_ty>)
        } else {
            quote!(#rust_ty)
        };
        sig.push(quote!(#attr #name: #ty));
        let var = format_ident!("__p{i}");
        match &kind {
            ParamKind::Prim(prim) => {
                let raw = match &p.default {
                    Some(d) => match scalar_default(d, &kind) {
                        Some(lit) => quote!(#name.unwrap_or(#lit)),
                        None => return Ok(Err(format!("default {d} does not fit a {prim:?}"))),
                    },
                    None => quote!(#name),
                };
                let what = format!("{} of `{}`", prim_name(*prim), item.name);
                let value = match prim {
                    Prim::Boolean | Prim::Int | Prim::Long => raw,
                    Prim::Byte => {
                        quote!(i8::try_from(#raw).map_err(|_| blazon_rt::Error::msg(format!("{} is out of range for a byte ({})", #what, #raw)))?)
                    }
                    Prim::Short => {
                        quote!(i16::try_from(#raw).map_err(|_| blazon_rt::Error::msg(format!("{} is out of range for a short ({})", #what, #raw)))?)
                    }
                    Prim::Char => quote!({
                        let s: &str = #raw;
                        let mut chars = s.chars();
                        match (chars.next(), chars.next()) {
                            (Some(c), None) if u32::from(c) <= 0xFFFF => c as u16,
                            _ => return Err(blazon_rt::Error::msg(format!("{} takes a one-character string in the basic multilingual plane, got {s:?}", #what))),
                        }
                    }),
                    Prim::Float => quote!((#raw).0 as f32),
                    Prim::Double => quote!((#raw).0),
                };
                prelude.push(quote!(let #var = #value;));
                call_args.push(quote!(#var));
            }
            ParamKind::Str => {
                let value = match &p.default {
                    Some(d) => match scalar_default(d, &kind) {
                        Some(lit) => quote!(#name.unwrap_or(#lit)),
                        None => return Ok(Err(format!("default {d} does not fit a string"))),
                    },
                    None => quote!(#name),
                };
                prelude.push(quote!(let #var: &str = #value;));
                call_args.push(quote!(#var));
            }
            ParamKind::Boxed(_) => {
                let value = match &p.default {
                    Some(d) => match scalar_default(d, &kind) {
                        Some(lit) => quote!(#name.unwrap_or(#lit)),
                        None => return Ok(Err(format!("default {d} does not fit"))),
                    },
                    None => quote!(#name),
                };
                prelude.push(quote!(let #var = #value;));
                call_args.push(quote!(#var));
            }
            ParamKind::Handle(r) => {
                let target = ref_arg(&r.concrete());
                let obj = match &p.default {
                    Some(d) => {
                        let dflt = default_object(d, rt);
                        quote!(match &#name {
                            Some(a) => Object::of(blazon_starlark::ToJava::to_java(a)?),
                            None => #dflt,
                        })
                    }
                    None => quote!(Object::of(blazon_starlark::ToJava::to_java(&#name)?)),
                };
                let typed = match r {
                    Ref::Handle { rust, .. } if rust == "Object" => quote!(#obj),
                    _ => quote!(#obj.downcast::<#target>()?),
                };
                prelude.push(quote!(let #var = #typed;));
                if m.sig.params[i].nullable {
                    call_args.push(quote!(Some(&#var)));
                } else {
                    call_args.push(quote!(&#var));
                }
            }
            ParamKind::Optional(_)
            | ParamKind::Bytes
            | ParamKind::StrArray
            | ParamKind::ObjArray { .. } => {
                return Ok(Err("a parameter of this kind has no Starlark bridge".into()));
            }
        }
    }
    let mut extra = item.params.len();
    if item.use_thread {
        prelude.push(quote!(let __thread = rt_thread(&mut env)?;));
        call_args.push(quote!(&__thread));
        extra += 1;
    }
    if item.use_semantics {
        prelude.push(quote!(let __sem = rt_semantics(&mut env)?;));
        call_args.push(quote!(&__sem));
        extra += 1;
    }
    if extra != m.sig.params.len() {
        return Err(anyhow!(
            "Bazel {label}: {}.{}: {} Starlark parameter(s) for {} Java parameter(s)",
            c.java,
            item.name,
            extra,
            m.sig.params.len()
        ));
    }
    let is_void = matches!(ret, RetKind::Void);
    let call = if matches!(m.kind, crate::ir::MemberKind::Field(_)) {
        quote!(__obj.#call_name()?)
    } else {
        quote!(__obj.#call_name(#(#call_args),*)?)
    };
    let nullable = m.sig.ret_nullable;
    let (ret_ty, convert) = match &ret {
        RetKind::Void => (
            quote!(starlark::values::none::NoneType),
            quote!(Ok(starlark::values::none::NoneType)),
        ),
        RetKind::Prim(Prim::Boolean) | RetKind::Boxed(Boxed::Bool) if !nullable => {
            (quote!(bool), quote!(Ok(__r)))
        }
        RetKind::Prim(Prim::Int) | RetKind::Boxed(Boxed::Int) if !nullable => {
            (quote!(i32), quote!(Ok(__r)))
        }
        RetKind::Prim(Prim::Long) | RetKind::Boxed(Boxed::Long) if !nullable => {
            (quote!(i64), quote!(Ok(__r)))
        }
        RetKind::Prim(Prim::Byte | Prim::Short) => (quote!(i32), quote!(Ok(i32::from(__r)))),
        RetKind::Prim(Prim::Char) => (quote!(String), quote!(Ok(String::from_utf16_lossy(&[__r])))),
        RetKind::Prim(Prim::Float) => (
            quote!(starlark::values::float::StarlarkFloat),
            quote!(Ok(starlark::values::float::StarlarkFloat(f64::from(__r)))),
        ),
        RetKind::Prim(Prim::Double) => (
            quote!(starlark::values::float::StarlarkFloat),
            quote!(Ok(starlark::values::float::StarlarkFloat(__r))),
        ),
        RetKind::Str if !nullable => (quote!(String), quote!(Ok(__r))),
        RetKind::Str | RetKind::Boxed(_) => (
            quote!(starlark::values::Value<'v>),
            quote!(Ok(match __r {
                Some(x) => heap.alloc(x),
                None => starlark::values::Value::new_none(),
            })),
        ),
        RetKind::Handle(_) if nullable => (
            quote!(starlark::values::Value<'v>),
            quote!(match __r {
                Some(x) => blazon_starlark::wrap(x.as_ref(), heap),
                None => Ok(starlark::values::Value::new_none()),
            }),
        ),
        RetKind::Handle(_) => (
            quote!(starlark::values::Value<'v>),
            quote!(blazon_starlark::wrap(__r.as_ref(), heap)),
        ),
        RetKind::Optional(_) => (
            quote!(starlark::values::Value<'v>),
            quote!(match __r {
                Some(x) => blazon_starlark::wrap(x.as_ref(), heap),
                None => Ok(starlark::values::Value::new_none()),
            }),
        ),
        RetKind::Prim(_) | RetKind::Bytes | RetKind::StrArray | RetKind::ObjArray(_) => {
            return Ok(Err("a result of this kind has no Starlark bridge".into()));
        }
    };
    let heap_used = convert.to_string().contains("heap");
    sig.push(if heap_used {
        quote!(heap: starlark::values::Heap<'v>)
    } else {
        quote!(_heap: starlark::values::Heap<'v>)
    });
    let bind = if is_void {
        quote!(#call;)
    } else {
        quote!(let __r = #call;)
    };
    let attribute = if item.attribute {
        quote!(#[starlark(attribute)])
    } else {
        quote!()
    };
    let docs = match &m.doc {
        Some(d) => {
            let d = format!(" {d}");
            quote!(#[doc = #d])
        }
        None => quote!(),
    };
    Ok(Ok(quote!(
        #docs
        #attribute
        fn #ident<'v>(#(#sig),*) -> starlark::Result<#ret_ty> {
            (|| -> blazon_rt::Result<#ret_ty> {
                let mut env = blazon_rt::vm::env()?;
                let __obj = this.obj.get(&mut env)?;
                #(#prelude)*
                #bind
                #convert
            })()
            .map_err(blazon_starlark::error)
        }
    )))
}

fn registry(classes: &[Class], rt: &Runtime) -> TokenStream {
    let mut faceted: Vec<&Class> = classes.iter().filter(|c| c.starlark.is_some()).collect();
    faceted.sort_by(|a, b| {
        let (fa, fb) = (a.starlark.as_ref(), b.starlark.as_ref());
        let (na, nb) = (
            fa.map_or(0, |f| f.ancestors.len()),
            fb.map_or(0, |f| f.ancestors.len()),
        );
        nb.cmp(&na).then(a.java.cmp(&b.java))
    });
    let checks = faceted.iter().map(|c| {
        let v = value_name(c, classes);
        let internal = &c.internal;
        quote!(
            {
                static CLASS: blazon_rt::member::ClassRef = blazon_rt::member::ClassRef::new(#internal);
                if CLASS.is_instance(&mut env, obj)? {
                    return Ok(Some(heap.alloc(#v::new(obj)?)));
                }
            }
        )
    });
    let unwraps = faceted.iter().map(|c| {
        let v = value_name(c, classes);
        quote!(
            if let Some(x) = v.downcast_ref::<#v>() {
                return Ok(Some(x.java_object()?));
            }
        )
    });
    let Runtime {
        starlark,
        none,
        int,
        int_of,
        int_to_big,
        ..
    } = rt;
    let to_big = int_to_big;
    quote!(
        /// The generated Starlark types of this version.
        pub struct Bridge;

        impl blazon_starlark::Registry for Bridge {
            fn wrap<'v>(&self, obj: &jni::objects::JObject, heap: starlark::values::Heap<'v>) -> blazon_rt::Result<Option<starlark::values::Value<'v>>> {
                let mut env = blazon_rt::vm::env()?;
                #(#checks)*
                Ok(None)
            }

            fn unwrap(&self, v: starlark::values::Value) -> blazon_rt::Result<Option<jni::objects::JObject<'static>>> {
                #(#unwraps)*
                Ok(None)
            }

            fn none(&self) -> blazon_rt::Result<jni::objects::JObject<'static>> {
                let mut env = blazon_rt::vm::env()?;
                Ok(#starlark::#none(&mut env)?.into_raw())
            }

            fn int(&self, v: i64) -> blazon_rt::Result<jni::objects::JObject<'static>> {
                let mut env = blazon_rt::vm::env()?;
                Ok(#int::#int_of(&mut env, v)?.into_raw())
            }

            fn int_value(&self, obj: &jni::objects::JObject) -> blazon_rt::Result<Option<i64>> {
                let env = blazon_rt::vm::env()?;
                let local = env.new_local_ref(obj)?;
                let i: #int = Object::of(local).downcast()?;
                Ok(i.#to_big()?.to_i64().ok())
            }
        }

        /// Registers this version's Starlark types with `blazon_starlark`.
        pub fn register() {
            blazon_starlark::register(&Bridge);
        }
    )
}
