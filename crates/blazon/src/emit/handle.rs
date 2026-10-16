//! Printing one class: its handle, the members, and the subtype relations.

use super::member::{self, class_static, id, ref_arg};
use super::types::{Bound, Scope};
use crate::ir::{Class, MemberKind, Target};
use proc_macro2::TokenStream;
use quote::quote;
use std::collections::BTreeMap;

pub fn class(
    c: &Class,
    bound: &BTreeMap<String, Bound>,
    label: &str,
    problems: &mut Vec<String>,
) -> TokenStream {
    let name = id(&c.rust);
    let internal = &c.internal;
    let static_class = class_static(c);
    let vars: Vec<proc_macro2::Ident> = c.type_params.iter().map(|v| id(v)).collect();
    let mut docs = vec![format!(" `{}`", c.java)];
    if !c.missing.is_empty() {
        let list: Vec<String> = c.missing.iter().map(|m| format!("`{m}`")).collect();
        docs.push(String::new());
        docs.push(format!(" Not found in Bazel {label}: {}.", list.join(", ")));
    }
    let scope = Scope {
        bound,
        class_vars: &c.type_params,
    };

    let mut instance = Vec::new();
    let mut statics = Vec::new();
    let mut ctors = Vec::new();
    for m in &c.members {
        match member::member(c, m, &scope, label) {
            Ok(tokens) => match m.kind {
                MemberKind::Method(Target::Instance) | MemberKind::Field(Target::Instance) => {
                    instance.push(tokens)
                }
                MemberKind::Constructor => ctors.push(tokens),
                _ => statics.push(tokens),
            },
            Err(e) => problems.push(format!("{e}")),
        }
    }
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
    let instance_impl = if instance.is_empty() {
        quote!()
    } else {
        quote!(impl<'a #bounded> #name<'a #generic_args> { #(#instance)* })
    };
    let ctor_impl = if ctors.is_empty() {
        quote!()
    } else {
        quote!(impl<'a #bounded> #name<'a #generic_args> { #(#ctors)* })
    };
    let static_impl = if statics.is_empty() {
        quote!()
    } else {
        quote!(impl<'a> #name<'a> { #(#statics)* })
    };

    let handle_macro = quote!(
        blazon_rt::handle!(
            #(#[doc = #docs])*
            #name,
            #internal
            #(, #vars)*
        );
    );

    let supers = c.supers.iter().filter_map(|s| {
        let b = bound.get(&s.internal)?;
        if s.internal == "java/lang/Object" || s.internal == "java/lang/Comparable" {
            return None;
        }
        let sup = id(&b.rust);
        let args: Vec<super::types::Ref> = s.args.iter().filter_map(|t| scope.reference_arg(t)).collect();
        let args: Vec<TokenStream> = if b.arity == 0 { Vec::new() } else { args.iter().map(ref_arg).collect() };
        Some(quote!(
            impl<'a #generic_args> blazon_rt::IsA<#sup<'static #(, #args)*>> for #name<'a #generic_args> {}

            impl<'a #generic_args> From<#name<'a #generic_args>> for #sup<'a #(, #args)*> {
                fn from(sub: #name<'a #generic_args>) -> Self {
                    // SAFETY: the class file says a `#name` is a `#sup`.
                    unsafe { blazon_rt::conv::upcast_owned(sub) }
                }
            }

            impl<'a #generic_args> TryFrom<#sup<'a #(, #args)*>> for #name<'a #generic_args> {
                type Error = blazon_rt::Error;

                fn try_from(sup: #sup<'a #(, #args)*>) -> blazon_rt::Result<Self> {
                    blazon_rt::checked_cast(sup)
                }
            }
        ))
    });

    let ord = if c.comparable {
        quote!(
            impl<'a, 'b #generic_args> PartialOrd<#name<'b #generic_args>> for #name<'a #generic_args> {
                fn partial_cmp(&self, other: &#name<'b #generic_args>) -> Option<std::cmp::Ordering> {
                    Some(blazon_rt::java::lang::compare_to(&self.obj, &other.obj))
                }
            }

            impl<'a #generic_args> Ord for #name<'a #generic_args> {
                fn cmp(&self, other: &Self) -> std::cmp::Ordering {
                    blazon_rt::java::lang::compare_to(&self.obj, &other.obj)
                }
            }
        )
    } else {
        quote!()
    };

    let class_ref = if c.members.iter().any(|m| m.constant.is_none()) {
        quote!(static #static_class: blazon_rt::member::ClassRef = blazon_rt::member::ClassRef::new(#internal);)
    } else {
        quote!()
    };

    quote!(
        #class_ref

        #handle_macro
        #instance_impl
        #ctor_impl
        #static_impl
        #(#supers)*
        #ord
    )
}
