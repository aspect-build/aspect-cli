extern crate proc_macro;

use std::collections::BTreeMap;

use anyhow::{Error, bail};
use darling::{FromField, FromMeta, FromVariant};
use proc_macro2::{Span, TokenStream};
use quote::{ToTokens, quote};
use syn::Item;
use syn::parse::Parser;
use syn::{Attribute, Field, ItemStruct, Lit, Meta, parse_str, spanned::Spanned};
use syn::{
    Data, DataEnum, DataStruct, DeriveInput, Expr, Fields, FieldsNamed, FieldsUnnamed, Ident, Type,
    Variant,
};

#[derive(FromField, FromVariant, Debug, Default)]
#[darling(attributes(prost))]
#[allow(dead_code)]
struct ProstAttrs {
    #[darling(default)]
    optional: bool,
    #[darling(default)]
    required: bool,
    #[darling(default)]
    repeated: bool,
    #[darling(default)]
    packed: bool,

    #[darling(default)]
    message: bool,
    #[darling(default)]
    uint32: bool,
    #[darling(default)]
    int32: bool,
    #[darling(default)]
    uint64: bool,
    #[darling(default)]
    int64: bool,
    #[darling(default)]
    bool: bool,
    #[darling(default)]
    string: bool,

    oneof: Option<String>,
    enumeration: Option<String>,
    map: Option<String>,
    tags: Option<String>,
    tag: Option<String>,
    default: Option<String>,
    bytes: Option<darling::util::Override<syn::Path>>,
}

#[derive(FromField, FromVariant, Debug, Default)]
#[darling(attributes(starbuf))]
struct StarbufAttrs {
    #[darling(default)]
    skip: bool,
    #[darling(default)]
    timestamp: bool,
    #[darling(default)]
    duration: bool,
    #[darling(default)]
    any: bool,

    #[allow(unused)]
    path: String,

    rename: Option<String>,
    return_type: Option<String>,
    return_expr: Option<String>,
}

impl From<syn::Ident> for ProstAttrs {
    fn from(_ident: syn::Ident) -> Self {
        ProstAttrs::default()
    }
}

fn try_types(input: TokenStream) -> Result<TokenStream, Error> {
    let input: Item = syn::parse2(input)?;

    fn traverse(
        prefix: TokenStream,
        defs: &mut BTreeMap<String, (Vec<TokenStream>, Vec<TokenStream>)>,
        item: &Item,
    ) {
        match item {
            Item::Mod(r#mod) if r#mod.content.is_some() => {
                let content = r#mod.content.as_ref().unwrap();
                for subitem in &content.1 {
                    let ident = &r#mod.ident;
                    let subpath = if prefix.is_empty() {
                        quote! { #ident }
                    } else {
                        quote! {#prefix::#ident}
                    };
                    let subpaths = subpath.to_string();

                    if let Item::Mod(smod) = subitem {
                        let subident = &smod.ident;
                        let subpath = quote! {#subpath::#subident};
                        let sub_key = subpath.to_string();
                        let subgenerator_fn = Ident::new(
                            &format!(
                                "{}_toplevels",
                                sub_key.replace("::", "_").replace(" ", "")
                            ),
                            Span::call_site(),
                        );
                        let subidentstr = subident.to_string();
                        defs.entry(subpaths)
                            .or_insert_with(|| (vec![], vec![]))
                            .1
                            .push(quote! { globals.namespace(#subidentstr, #subgenerator_fn); });
                        // Ensure the sub-module entry exists even if it has no processable items
                        defs.entry(sub_key).or_insert_with(|| (vec![], vec![]));
                    }

                    traverse(subpath, defs, &subitem)
                }
            }
            Item::Enum(_) => {
                // Generate empty entry to allow generation of the toplevels function below.
                defs.entry(prefix.to_string())
                    .or_insert_with(|| (vec![], vec![]))
                    .0
                    .push(quote! {});
            }
            Item::Struct(st) if st.generics.params.is_empty() => {
                let ident = &st.ident;
                let subpaths = prefix.to_string();
                defs.entry(subpaths.clone())
                    .or_insert_with(|| (vec![], vec![]))
                    .0
                    .push(quote! {
                        const #ident: ::starlark::values::starlark_value_as_type::StarlarkValueAsType<#prefix::#ident> =
                        starlark::values::starlark_value_as_type::StarlarkValueAsType::new();
                    });
                let ident_snake = snake(ident.to_string());
                let constructor_fn = Ident::new(
                    &format!("{}_constructor", ident_snake),
                    ident.span(),
                );
                defs.entry(subpaths)
                    .or_insert_with(|| (vec![], vec![]))
                    .1
                    .push(quote! {
                        #prefix::#constructor_fn(globals);
                    });
            }
            _ => {}
        };
    }

    let mut defs: BTreeMap<String, (Vec<TokenStream>, Vec<TokenStream>)> = BTreeMap::new();

    traverse(TokenStream::new(), &mut defs, &input);

    let registers = defs.iter().map(|(k, (defs, inherit))| {
        let ident_types = Ident::new(
            &format!("{}_types", k.replace("::", "_").replace(" ", "")),
            Span::call_site(),
        );

        let ident = Ident::new(
            &format!(
                "{}_toplevels",
                k.to_string().replace("::", "_").replace(" ", "")
            ),
            Span::call_site(),
        );

        quote! {
            #[::starlark::starlark_module]
            fn #ident_types(globals: &mut ::starlark::environment::GlobalsBuilder) {
                 #(#defs)*
            }
            pub fn #ident(globals: &mut ::starlark::environment::GlobalsBuilder) {
                 #ident_types(globals);
                #(#inherit)*
            }
        }
    });

    let expanded = quote! {

        #(#registers)*

        #input
    };

    Ok(expanded)
}

struct ServiceRpc {
    name: Ident,
    method: Ident,
    request: Type,
    response: Type,
}

fn parse_service_attr(attr: TokenStream) -> Result<(Type, Vec<ServiceRpc>), Error> {
    let args: syn::punctuated::Punctuated<Meta, syn::Token![,]> =
        syn::punctuated::Punctuated::parse_terminated.parse2(attr)?;

    let mut client: Option<Type> = None;
    let mut methods: Vec<ServiceRpc> = Vec::new();

    for arg in args {
        match arg {
            Meta::NameValue(nv) if nv.path.is_ident("client") => {
                let syn::Expr::Lit(expr_lit) = &nv.value else {
                    bail!("client must be a string literal type path");
                };
                let Lit::Str(lit) = &expr_lit.lit else {
                    bail!("client must be a string literal type path");
                };
                let ty: Type = parse_str(&lit.value())?;
                client = Some(ty);
            }
            Meta::List(list) if list.path.is_ident("methods") => {
                let mut name: Option<Ident> = None;
                let mut method: Option<Ident> = None;
                let mut request: Option<Type> = None;
                let mut response: Option<Type> = None;

                let nested: syn::punctuated::Punctuated<Meta, syn::Token![,]> =
                    syn::punctuated::Punctuated::parse_terminated.parse2(list.tokens.clone())?;

                for nested_meta in nested.iter() {
                    match nested_meta {
                        Meta::NameValue(nv) if nv.path.is_ident("name") => {
                            let syn::Expr::Lit(expr_lit) = &nv.value else {
                                bail!("method name must be a string literal");
                            };
                            let Lit::Str(lit) = &expr_lit.lit else {
                                bail!("method name must be a string literal");
                            };
                            name = Some(Ident::new(&lit.value(), lit.span()));
                        }
                        Meta::NameValue(nv) if nv.path.is_ident("method") => {
                            let syn::Expr::Lit(expr_lit) = &nv.value else {
                                bail!("tonic method must be a string literal");
                            };
                            let Lit::Str(lit) = &expr_lit.lit else {
                                bail!("tonic method must be a string literal");
                            };
                            method = Some(Ident::new(&lit.value(), lit.span()));
                        }
                        Meta::NameValue(nv) if nv.path.is_ident("request") => {
                            let syn::Expr::Lit(expr_lit) = &nv.value else {
                                bail!("request must be a string literal type path");
                            };
                            let Lit::Str(lit) = &expr_lit.lit else {
                                bail!("request must be a string literal type path");
                            };
                            request = Some(parse_str(&lit.value())?);
                        }
                        Meta::NameValue(nv) if nv.path.is_ident("response") => {
                            let syn::Expr::Lit(expr_lit) = &nv.value else {
                                bail!("response must be a string literal type path");
                            };
                            let Lit::Str(lit) = &expr_lit.lit else {
                                bail!("response must be a string literal type path");
                            };
                            response = Some(parse_str(&lit.value())?);
                        }
                        _ => {}
                    }
                }
                let Some(name) = name else {
                    bail!("each methods(...) entry must include name");
                };
                let Some(method) = method else {
                    bail!("each methods(...) entry must include method");
                };
                let Some(request) = request else {
                    bail!("each methods(...) entry must include request");
                };
                let Some(response) = response else {
                    bail!("each methods(...) entry must include response");
                };

                methods.push(ServiceRpc {
                    name,
                    method,
                    request,
                    response,
                });
            }
            _ => {}
        }
    }

    let client = client
        .ok_or_else(|| anyhow::anyhow!("service attribute requires client = \"path::Type\""))?;
    if methods.is_empty() {
        bail!("service attribute requires at least one methods(...) entry");
    }

    Ok((client, methods))
}

#[proc_macro_attribute]
pub fn types(
    _attr: proc_macro::TokenStream,
    input: proc_macro::TokenStream,
) -> proc_macro::TokenStream {
    try_types(input.into()).unwrap().into()
}

fn try_message(input: TokenStream) -> Result<TokenStream, Error> {
    let input: DeriveInput = syn::parse2(input)?;
    let ident = input.ident;

    let variant_data = match input.data {
        Data::Struct(variant_data) => variant_data,
        Data::Enum(..) => bail!("message can not be derived for an enum"),
        Data::Union(..) => bail!("message can not be derived for a union"),
    };

    let (_, fields) = match variant_data {
        DataStruct {
            fields: Fields::Named(FieldsNamed { named: fields, .. }),
            ..
        } => (true, fields.into_iter().collect()),
        DataStruct {
            fields:
                Fields::Unnamed(FieldsUnnamed {
                    unnamed: fields, ..
                }),
            ..
        } => (false, fields.into_iter().collect()),
        DataStruct {
            fields: Fields::Unit,
            ..
        } => (false, Vec::new()),
    };

    let fields: Vec<(&Field, StarbufAttrs, ProstAttrs, Vec<Attribute>)> = fields
        .iter()
        .map(|f| {
            let mut doc_attrs = Vec::new();

            // Iterate through the input struct's attributes to find doc comments
            for attr in &f.attrs {
                // Check if the attribute path is "doc"
                if attr.path().is_ident("doc") {
                    // Extract the string literal from the `#[doc = "value"]` attribute
                    if let syn::Meta::NameValue(nv) = &attr.meta {
                        if let syn::Expr::Lit(expr_lit) = &nv.value {
                            if let syn::Lit::Str(lit_str) = &expr_lit.lit {
                                let doc_value = lit_str.value();
                                // Format each line as a `///` comment
                                let new_doc_attr = Attribute {
                                    pound_token: syn::token::Pound::default(),
                                    style: syn::AttrStyle::Outer,
                                    bracket_token: syn::token::Bracket::default(),
                                    meta: syn::Meta::NameValue(syn::MetaNameValue {
                                        path: syn::Path::from(syn::Ident::new("doc", attr.span())),
                                        eq_token: syn::token::Eq::default(),
                                        value: syn::Expr::Lit(syn::ExprLit {
                                            attrs: Vec::new(),
                                            lit: syn::Lit::Str(syn::LitStr::new(
                                                &doc_value,
                                                lit_str.span(),
                                            )),
                                        }),
                                    }),
                                };
                                doc_attrs.push(new_doc_attr);
                            }
                        }
                    }
                }
            }

            (
                f,
                StarbufAttrs::from_field(f).expect("failed to parse"),
                ProstAttrs::from_field(f).expect("failed to parse prost attributes"),
                doc_attrs,
            )
        })
        .collect();

    let starlark_attributes = fields.iter().map(|(field, sattrs, attrs, docs)| {
        let has_deprecated = field.attrs.iter().any(|v| v.path().is_ident("deprecated"));
        if sattrs.skip || has_deprecated {
            return quote! {};
        }
        let new_return_type = if let Type::Path(p) = &field.ty {
            let tys = p
                .path
                .segments
                .iter()
                .map(|i| i.ident.to_string())
                .collect::<Vec<String>>()
                .join("::");

            let args = &p.path.segments.last().unwrap().arguments;
            if tys == "core::option::Option" {
                let ty: Type = parse_str(
                    format!(
                        "::starlark::values::none::NoneOr{}",
                        args.to_token_stream().to_string()
                    )
                    .as_str(),
                )
                .unwrap();
                Some(ty)
            } else if tys == "prost::alloc::vec::Vec" {
                let ty: Type = parse_str(
                    format!(
                        "::starlark::values::list::AllocList<{}>",
                        p.to_token_stream().to_string()
                    )
                    .as_str(),
                )
                .unwrap();
                Some(ty)
            } else if tys == "std::collections::HashMap" {
                let ty: Type = parse_str(
                    format!(
                        "::starlark::values::dict::AllocDict<{}>",
                        p.to_token_stream().to_string()
                    )
                    .as_str(),
                )
                .unwrap();
                Some(ty)
            } else {
                None
            }
        } else {
            None
        };

        if sattrs.any || sattrs.duration || sattrs.timestamp || sattrs.skip || attrs.bytes.is_some()
        {
            return quote! {};
        }

        let id_ty: Option<Type> = sattrs.return_type.as_ref().map(|ty| parse_str(ty).unwrap());

        let fident = if sattrs.rename.is_some() {
            Ident::from_string(sattrs.rename.as_ref().unwrap()).unwrap()
        } else {
            field.ident.clone().unwrap()
        };

        let return_type = if sattrs.return_type.is_some() {
            &id_ty.unwrap()
        } else if attrs.optional {
            &new_return_type.unwrap()
        } else if attrs.repeated {
            &new_return_type.unwrap()
        } else if attrs.map.is_some() {
            &new_return_type.unwrap()
        } else if attrs.oneof.is_some() {
            &new_return_type.unwrap()
        } else if attrs.enumeration.is_some() {
            &Type::from_string(&attrs.enumeration.as_ref().unwrap())
                .expect("failed to parse enum type")
        } else {
            &field.ty
        };

        let return_expr = if sattrs.return_expr.is_some() {
            let expr: TokenStream =
                syn::parse_str(sattrs.return_expr.as_ref().unwrap().as_str()).unwrap();
            quote! { Ok(#expr) }
        } else if attrs.optional {
            quote! { Ok(::starlark::values::none::NoneOr::from_option(this.#fident.clone())) }
        } else if attrs.repeated {
            quote! { Ok(::starlark::values::list::AllocList(this.#fident.clone())) }
        } else if attrs.oneof.is_some() {
            quote! { Ok(::starlark::values::none::NoneOr::from_option(this.#fident.clone())) }
        } else if attrs.map.is_some() {
            quote! { Ok(::starlark::values::dict::AllocDict(this.#fident.clone())) }
        } else if attrs.enumeration.is_some() {
            let ty = Type::from_string(&attrs.enumeration.as_ref().unwrap())
                .expect("failed to parse enum type");
            quote! { Ok(#ty::try_from(this.#fident)?) }
        } else {
            quote! { Ok(this.#fident.clone()) }
        };

        quote! {
            #(#docs)*
            #[starlark(attribute)]
            fn #fident<'v>(this: ::starlark::values::Value<'v>) -> ::anyhow::Result<#return_type> {
                use ::starlark::values::ValueLike;
                let this = this.downcast_ref_err::<#ident>()?;
                #return_expr
            }
        }
    });

    let ident_snake = snake(ident.to_string());
    let methods_ident = Ident::new(&format!("{}_methods", &ident_snake), ident.span());
    let constructor_fn_ident = Ident::new(&format!("{}_constructor", &ident_snake), ident.span());

    let constructor_arms: Vec<TokenStream> = fields
        .iter()
        .filter_map(|(field, sattrs, attrs, _)| {
            let has_deprecated = field.attrs.iter().any(|v| v.path().is_ident("deprecated"));
            if sattrs.skip
                || has_deprecated
                || sattrs.any
                || sattrs.duration
                || sattrs.timestamp
                || attrs.bytes.is_some()
                || attrs.map.is_some()
                || attrs.oneof.is_some()
            {
                return None;
            }

            let fident = field.ident.as_ref()?;
            let fident_str = fident.to_string();

            let conversion = if attrs.optional && attrs.message {
                let inner_ty = extract_inner_type(&field.ty, "core::option::Option")?;
                let inner_str = inner_ty.to_token_stream().to_string().replace(' ', "");
                if matches!(inner_str.as_str(), "u32" | "i32" | "u64" | "i64" | "f32" | "f64" | "bool") {
                    // prost wraps proto3 optional scalars as message+optional — skip in constructor
                    return None;
                }
                quote! {
                    use ::starlark::values::ValueLike;
                    result.#fident = Some(value.downcast_ref_err::<#inner_ty>()?.clone());
                }
            } else if attrs.optional && attrs.string {
                quote! {
                    result.#fident = Some(value.unpack_str()
                        .ok_or_else(|| ::anyhow::anyhow!("field '{}' expects a string", #fident_str))?
                        .to_string());
                }
            } else if attrs.optional && (attrs.int32 || attrs.uint32 || attrs.int64 || attrs.uint64) {
                quote! {
                    result.#fident = Some(<i32 as ::starlark::values::UnpackValue>::unpack_value(value)?
                        .ok_or_else(|| ::anyhow::anyhow!("expected int"))? as _);
                }
            } else if attrs.optional && attrs.bool {
                quote! {
                    result.#fident = Some(value.unpack_bool()
                        .ok_or_else(|| ::anyhow::anyhow!("field '{}' expects a bool", #fident_str))?);
                }
            } else if attrs.optional && attrs.enumeration.is_some() {
                quote! {
                    result.#fident = Some(<i32 as ::starlark::values::UnpackValue>::unpack_value(value)?
                        .ok_or_else(|| ::anyhow::anyhow!("expected int"))? as i32);
                }
            } else if attrs.repeated && attrs.message {
                let inner_ty = extract_inner_type(&field.ty, "prost::alloc::vec::Vec")?;
                quote! {
                    use ::starlark::values::ValueLike;
                    let list = ::starlark::values::list::ListRef::from_value(value)
                        .ok_or_else(|| ::anyhow::anyhow!("field '{}' expects a list", #fident_str))?;
                    for item in list.iter() {
                        result.#fident.push(item.downcast_ref_err::<#inner_ty>()?.clone());
                    }
                }
            } else if attrs.repeated && attrs.string {
                quote! {
                    let list = ::starlark::values::list::ListRef::from_value(value)
                        .ok_or_else(|| ::anyhow::anyhow!("field '{}' expects a list", #fident_str))?;
                    for item in list.iter() {
                        result.#fident.push(item.unpack_str()
                            .ok_or_else(|| ::anyhow::anyhow!("list items for '{}' must be strings", #fident_str))?
                            .to_string());
                    }
                }
            } else if attrs.repeated && (attrs.int32 || attrs.uint32 || attrs.int64 || attrs.uint64) {
                quote! {
                    let list = ::starlark::values::list::ListRef::from_value(value)
                        .ok_or_else(|| ::anyhow::anyhow!("field '{}' expects a list", #fident_str))?;
                    for item in list.iter() {
                        result.#fident.push(<i32 as ::starlark::values::UnpackValue>::unpack_value(item)?
                            .ok_or_else(|| ::anyhow::anyhow!("expected int"))? as _);
                    }
                }
            } else if attrs.repeated && attrs.enumeration.is_some() {
                quote! {
                    let list = ::starlark::values::list::ListRef::from_value(value)
                        .ok_or_else(|| ::anyhow::anyhow!("field '{}' expects a list", #fident_str))?;
                    for item in list.iter() {
                        result.#fident.push(<i32 as ::starlark::values::UnpackValue>::unpack_value(item)?
                            .ok_or_else(|| ::anyhow::anyhow!("expected int"))? as i32);
                    }
                }
            } else if attrs.string {
                quote! {
                    result.#fident = value.unpack_str()
                        .ok_or_else(|| ::anyhow::anyhow!("field '{}' expects a string", #fident_str))?
                        .to_string();
                }
            } else if attrs.bool {
                quote! {
                    result.#fident = value.unpack_bool()
                        .ok_or_else(|| ::anyhow::anyhow!("field '{}' expects a bool", #fident_str))?;
                }
            } else if attrs.int32 || attrs.uint32 {
                quote! {
                    result.#fident = <i32 as ::starlark::values::UnpackValue>::unpack_value(value)?
                        .ok_or_else(|| ::anyhow::anyhow!("expected int"))? as _;
                }
            } else if attrs.int64 || attrs.uint64 {
                quote! {
                    result.#fident = <i32 as ::starlark::values::UnpackValue>::unpack_value(value)?
                        .ok_or_else(|| ::anyhow::anyhow!("expected int"))? as _;
                }
            } else if attrs.enumeration.is_some() {
                quote! {
                    result.#fident = <i32 as ::starlark::values::UnpackValue>::unpack_value(value)?
                        .ok_or_else(|| ::anyhow::anyhow!("expected int"))? as i32;
                }
            } else if attrs.message {
                let ty = &field.ty;
                quote! {
                    use ::starlark::values::ValueLike;
                    result.#fident = value.downcast_ref_err::<#ty>()?.clone();
                }
            } else {
                return None;
            };

            Some(quote! {
                #fident_str => { #conversion },
            })
        })
        .collect();

    let expanded = quote! {
        impl<'v> ::starlark::values::AllocValue<'v> for #ident {
            fn alloc_value(self, heap: &'v ::starlark::values::Heap) -> ::starlark::values::Value<'v> {
                heap.alloc_simple(self)
            }
        }

        #[::starlark::values::starlark_value(type = #ident_snake)]
        impl<'v> ::starlark::values::StarlarkValue<'v> for #ident {
            fn get_methods() -> ::core::option::Option<&'static ::starlark::environment::Methods> {
                static RES: ::starlark::environment::MethodsStatic =
                    ::starlark::environment::MethodsStatic::new();
                RES.methods(#methods_ident)
            }
        }

        #[::starlark::starlark_module]
        pub(crate) fn #methods_ident(registry: &mut ::starlark::environment::MethodsBuilder) {
            #(#starlark_attributes)*
        }

        #[::starlark::starlark_module]
        pub fn #constructor_fn_ident(globals: &mut ::starlark::environment::GlobalsBuilder) {
            fn #ident<'v>(
                #[starlark(kwargs)] kwargs: ::starlark::collections::SmallMap<&str, ::starlark::values::Value<'v>>,
            ) -> ::starlark::Result<#ident> {
                let mut result = #ident::default();
                for (key, val) in kwargs.iter() {
                    let value = *val;
                    match *key {
                        #(#constructor_arms)*
                        other => return Err(::anyhow::anyhow!(
                            "unknown field '{}' for {}", other, stringify!(#ident)
                        ).into()),
                    }
                }
                Ok(result)
            }
        }
    };

    Ok(expanded)
}

#[proc_macro_derive(Message, attributes(prost, starbuf))]
pub fn message(input: proc_macro::TokenStream) -> proc_macro::TokenStream {
    try_message(input.into()).unwrap().into()
}

fn try_oneof(input: TokenStream) -> Result<TokenStream, Error> {
    let input: DeriveInput = syn::parse2(input)?;

    let ident = input.ident;

    let variants = match input.data {
        Data::Enum(DataEnum { variants, .. }) => variants,
        Data::Struct(..) => bail!("oneof can not be derived for a struct"),
        Data::Union(..) => bail!("oneof can not be derived for a union"),
    };

    let mut fields: Vec<(FieldsUnnamed, Ident, ProstAttrs)> = Vec::new();

    for variant in variants {
        let attrs = ProstAttrs::from_variant(&variant).expect("failed to parse attributes");
        match &variant.fields {
            Fields::Unnamed(un) => fields.push((un.clone(), variant.ident.clone(), attrs)),
            _ => continue,
        };
    }

    let starlark_types = fields.iter().map(|(field, _, attrs)| {
        if attrs.string {
            quote! { ::starlark::typing::Ty::string() }
        } else if attrs.int32 || attrs.int64 || attrs.uint32 || attrs.uint64 {
            quote! { ::starlark::typing::Ty::int() }
        } else if attrs.bytes.is_some() {
            quote! { ::starlark::typing::Ty::string() }
        } else {
            let ty = &field.unnamed;
            quote! {
                ::starlark::typing::Ty::starlark_value::<#ty>()
            }
        }
    });

    let alloc = fields.iter().map(|(_, variant_ident, attrs)| {
        if attrs.string {
            quote! {
                Self::#variant_ident(value) =>  {
                    use starlark::values::ValueLike;
                    heap.alloc_str(value.as_str()).to_value()
                }
            }
        } else if attrs.bytes.is_some() {
            quote! {
                Self::#variant_ident(value) => {
                    use starlark::values::ValueLike;
                    heap.alloc(heap.alloc_str(
                        unsafe { ::std::string::String::from_utf8_unchecked(value.clone()) }.as_str(),
                    )).to_value()
                }
            }
        } else {
            quote! {
                Self::#variant_ident(value) => heap.alloc(value)
            }
        }
    });

    let expanded = quote! {
        impl starlark::values::type_repr::StarlarkTypeRepr for #ident {
            type Canonical = Self;

            fn starlark_type_repr() -> ::starlark::typing::Ty {
                ::starlark::typing::Ty::unions(vec![#(#starlark_types,)*])
            }
        }

        impl<'v> ::starlark::values::AllocValue<'v> for #ident {
            fn alloc_value(self, heap: &'v starlark::values::Heap) -> ::starlark::values::Value<'v> {
                match self {
                    #(#alloc,)*
                }
            }
        }
    };

    Ok(expanded)
}

#[proc_macro_derive(Oneof, attributes(prost))]
pub fn oneof(input: proc_macro::TokenStream) -> proc_macro::TokenStream {
    try_oneof(input.into()).unwrap().into()
}

fn extract_inner_type(ty: &Type, expected_path: &str) -> Option<Type> {
    if let Type::Path(p) = ty {
        let tys = p
            .path
            .segments
            .iter()
            .map(|i| i.ident.to_string())
            .collect::<Vec<_>>()
            .join("::");
        if tys == expected_path {
            if let Some(last_seg) = p.path.segments.last() {
                if let syn::PathArguments::AngleBracketed(args) = &last_seg.arguments {
                    if let Some(syn::GenericArgument::Type(inner_ty)) = args.args.first() {
                        return Some(inner_ty.clone());
                    }
                }
            }
        }
    }
    None
}

fn snake(s: String) -> String {
    let mut result = String::new();
    let mut chars = s.chars().peekable();

    while let Some(c) = chars.next() {
        if c.is_ascii_uppercase() {
            // Add an underscore before a new word, unless it's the very first character
            if !result.is_empty() && result.chars().last().unwrap() != '_' {
                result.push('_');
            }
            result.push(c.to_ascii_lowercase());
        } else {
            result.push(c);
        }
    }
    result
}

fn try_enumeration(input: TokenStream) -> Result<TokenStream, Error> {
    let input: DeriveInput = syn::parse2(input)?;
    let ident = input.ident;

    let punctuated_variants = match input.data {
        Data::Enum(DataEnum { variants, .. }) => variants,
        Data::Struct(_) => bail!("enumeration can not be derived for a struct"),
        Data::Union(..) => bail!("enumeration can not be derived for a union"),
    };

    // Map the variants into 'fields'.
    let mut variants: Vec<(Ident, Expr)> = Vec::new();
    for Variant {
        ident,
        fields,
        discriminant,
        ..
    } in punctuated_variants
    {
        match fields {
            Fields::Unit => (),
            Fields::Named(_) | Fields::Unnamed(_) => {
                bail!("enumeration variants may not have fields")
            }
        }
        match discriminant {
            Some((_, expr)) => variants.push((ident, expr)),
            None => bail!("enumeration variants must have a discriminant"),
        }
    }

    let alloc = variants.iter().map(|(ident, _expr)| {
        let value = snake(ident.to_string());
        quote! {
            Self::#ident => heap.alloc_str(#value).to_value()
        }
    });

    let expanded = quote! {
        impl starlark::values::type_repr::StarlarkTypeRepr for #ident {
            type Canonical = Self;

            fn starlark_type_repr() -> ::starlark::typing::Ty {
                ::starlark::typing::Ty::string()
            }
        }

        impl<'v> ::starlark::values::AllocValue<'v> for #ident {
            fn alloc_value(self, heap: &'v starlark::values::Heap) -> starlark::values::Value<'v> {
                match self {
                    #(#alloc,)*
                }
            }
        }
    };

    Ok(expanded)
}

#[proc_macro_derive(Enumeration, attributes(prost))]
pub fn enumeration(input: proc_macro::TokenStream) -> proc_macro::TokenStream {
    try_enumeration(input.into()).unwrap().into()
}

fn try_service(attr: TokenStream, item: TokenStream) -> Result<TokenStream, Error> {
    let (client_ty, methods) = parse_service_attr(attr)?;

    let input: ItemStruct = syn::parse2(item)?;
    let ident = &input.ident;

    let handle_ident = Ident::new(&format!("{}ClientHandle", ident), ident.span());
    let ident_snake = snake(ident.to_string());
    let methods_ident = Ident::new(&format!("{}_client_methods", ident_snake), ident.span());
    let module_ident = Ident::new(&format!("{}_service", ident_snake), ident.span());
    let starlark_type = format!("{}_client", ident_snake);

    let rpc_methods = methods.iter().map(|rpc| {
        let rpc_name = &rpc.name;
        let rpc_method = &rpc.method;
        let req = &rpc.request;
        let resp = &rpc.response;

        quote! {
            fn #rpc_name<'v>(
                this: ::starlark::values::Value<'v>,
                req: ::starlark::values::Value<'v>,
            ) -> ::starlark::Result<#resp> {
                use ::starlark::values::ValueLike;
                let handle = this.downcast_ref_err::<#handle_ident>()?;
                let req = req.downcast_ref_err::<#req>()?.clone();

                let client = handle.client.get()
                    .ok_or_else(|| ::starlark::Error::from(::anyhow::anyhow!(
                        "service not connected; call .connect(ctx) first")))?
                    .clone();
                let rt = handle.rt.get()
                    .ok_or_else(|| ::starlark::Error::from(::anyhow::anyhow!(
                        "service not connected; call .connect(ctx) first")))?
                    .clone();

                let headers = handle.headers.clone();

                let resp = rt.block_on(async move {
                    let mut c = client.as_ref().clone();
                    let mut request = ::tonic::Request::new(req);
                    for (key, value) in &headers {
                        request.metadata_mut().insert(
                            key.parse::<::tonic::metadata::MetadataKey<::tonic::metadata::Ascii>>()
                                .map_err(|e| ::anyhow::anyhow!("invalid header key '{}': {}", key, e))?,
                            value.parse::<::tonic::metadata::MetadataValue<::tonic::metadata::Ascii>>()
                                .map_err(|e| ::anyhow::anyhow!("invalid header value: {}", e))?,
                        );
                    }
                    let resp = c
                        .#rpc_method(request)
                        .await
                        .map_err(::anyhow::Error::new)?
                        .into_inner();
                    Ok::<#resp, ::anyhow::Error>(resp)
                })
                .map_err(|e| ::starlark::Error::from(::anyhow::anyhow!(e)))?;

                Ok(resp)
            }
        }
    });

    let expanded = quote! {
        #[derive(Debug, ::allocative::Allocative, ::starlark::values::NoSerialize, ::starlark::values::ProvidesStaticType)]
        pub struct #handle_ident {
            uri: String,
            headers: Vec<(String, String)>,
            timeout: ::std::time::Duration,
            #[allocative(skip)]
            client: ::std::sync::OnceLock<::std::sync::Arc<#client_ty<::tonic::transport::Channel>>>,
            #[allocative(skip)]
            rt: ::std::sync::OnceLock<::tokio::runtime::Handle>,
        }

        unsafe impl<'v> ::starlark::values::Trace<'v> for #handle_ident {
            fn trace(&mut self, _tracer: &::starlark::values::Tracer<'v>) {}
        }

        impl ::std::fmt::Display for #handle_ident {
            fn fmt(&self, f: &mut ::std::fmt::Formatter<'_>) -> ::std::fmt::Result {
                write!(f, stringify!(#handle_ident))
            }
        }

        impl<'v> ::starlark::values::AllocValue<'v> for #handle_ident {
            fn alloc_value(self, heap: &'v ::starlark::values::Heap) -> ::starlark::values::Value<'v> {
                heap.alloc_simple(self)
            }
        }

        #[::starlark::values::starlark_value(type = #starlark_type)]
        impl<'v> ::starlark::values::StarlarkValue<'v> for #handle_ident {
            fn get_methods() -> ::core::option::Option<&'static ::starlark::environment::Methods> {
                static RES: ::starlark::environment::MethodsStatic = ::starlark::environment::MethodsStatic::new();
                RES.methods(#methods_ident)
            }
        }

        #[::starlark::starlark_module]
        pub(crate) fn #methods_ident(registry: &mut ::starlark::environment::MethodsBuilder) {
            fn connect<'v>(
                this: ::starlark::values::Value<'v>,
                ctx: ::starlark::values::Value<'v>,
            ) -> ::starlark::Result<::starlark::values::none::NoneType> {
                use ::starlark::values::ValueLike;
                let handle = this.downcast_ref_err::<#handle_ident>()?;

                // Normalize grpcs:// to https://
                let uri = if handle.uri.starts_with("grpcs://") {
                    format!("https://{}", &handle.uri["grpcs://".len()..])
                } else {
                    handle.uri.clone()
                };

                let rt = ::tokio::runtime::Handle::current();

                let ep = ::tonic::transport::Endpoint::from_shared(uri.clone())
                    .map_err(|e| ::anyhow::anyhow!("invalid URI: {}", e))?
                    .connect_timeout(::std::time::Duration::from_secs(5))
                    .timeout(handle.timeout);

                let ep = if uri.starts_with("https://") {
                    ep.tls_config(::tonic::transport::ClientTlsConfig::new())
                        .map_err(|e| ::anyhow::anyhow!("TLS config error: {}", e))?
                } else {
                    ep
                };

                let channel = ep.connect_lazy();
                let client = #client_ty::new(channel);

                handle.client.set(::std::sync::Arc::new(client))
                    .map_err(|_| ::anyhow::anyhow!("service already connected"))?;
                handle.rt.set(rt)
                    .map_err(|_| ::anyhow::anyhow!("service already connected"))?;

                Ok(::starlark::values::none::NoneType)
            }

            #(#rpc_methods)*
        }

        #[::starlark::starlark_module]
        pub fn #module_ident(globals: &mut ::starlark::environment::GlobalsBuilder) {
            fn #ident<'v>(
                #[starlark(require = named)] uri: String,
                #[starlark(require = named)] headers: ::starlark::values::Value<'v>,
                #[starlark(require = named, default = 10000)] timeout: u64,
            ) -> ::starlark::Result<#handle_ident> {
                let mut h = Vec::new();
                if let Some(dict) = ::starlark::values::dict::DictRef::from_value(headers) {
                    for (k, v) in dict.iter() {
                        let key = k.unpack_str()
                            .ok_or_else(|| ::anyhow::anyhow!("header key must be a string"))?;
                        let val = v.unpack_str()
                            .ok_or_else(|| ::anyhow::anyhow!("header value must be a string"))?;
                        h.push((key.to_string(), val.to_string()));
                    }
                }
                Ok(#handle_ident {
                    uri,
                    headers: h,
                    timeout: ::std::time::Duration::from_millis(timeout),
                    client: ::std::sync::OnceLock::new(),
                    rt: ::std::sync::OnceLock::new(),
                })
            }
        }

        #input
    };

    Ok(expanded)
}

#[proc_macro_attribute]
pub fn service(
    attr: proc_macro::TokenStream,
    item: proc_macro::TokenStream,
) -> proc_macro::TokenStream {
    try_service(attr.into(), item.into()).unwrap().into()
}

#[cfg(test)]
mod tests {
    use super::*;
    // #[test]
    // fn try_types_test() {
    //     let output = try_types(quote! {
    //         pub mod build_event_stream {

    //             pub struct BuildEvent {}

    //             pub mod build_event_id {
    //                 pub struct UnknownBuildEventId {}
    //                 pub struct UnknownBuildEventId2 {}
    //                 pub enum ExecutionInfo {}
    //             }

    //             pub mod test_result {
    //                 pub struct ExecutionInfo {}
    //                 pub enum Ignore {}
    //             }

    //         }
    //     });
    //     let out = output.unwrap().to_string();
    //     eprintln!("{}", out);
    //     let file = syn::parse_file(out.as_str()).unwrap();
    //     eprintln!("{}", prettyplease::unparse(&file));
    //     assert!(false);
    // }

    // #[test]
    // fn try_oneof_test() {
    //     let output = try_oneof(quote! {
    //         pub enum File {
    //             #[prost(string, tag = "2")]
    //             Uri(::prost::alloc::string::String),
    //             #[prost(bytes, tag = "3")]
    //             Contents(::prost::alloc::vec::Vec<u8>),
    //             #[prost(string, tag = "7")]
    //             SymlinkTargetPath(::prost::alloc::string::String),
    //         }
    //     });
    //     let file = syn::parse_file(output.unwrap().to_string().as_str()).unwrap();
    //     eprintln!("{}", prettyplease::unparse(&file));
    //     assert!(false);
    // }

    // #[test]
    // fn try_oneof_with_bytes() {
    //     let output = try_oneof(quote! {
    //         pub enum ReplacementValue {
    //             #[prost(string, tag = "3")]
    //             NewValue(::prost::alloc::string::String),
    //             #[prost(message, tag = "4")]
    //             UseDefault(super::UseDefault),
    //         }
    //     });
    //     let file = syn::parse_file(output.unwrap().to_string().as_str()).unwrap();
    //     eprintln!("{}", prettyplease::unparse(&file));
    //     assert!(false);
    // }

    // #[test]
    // fn try_enum_test() {
    //     let output = try_enumeration(quote! {
    //         #[derive(::prost::Enumeration)]
    //         #[repr(i32)]
    //         pub enum AbortReason {
    //             Unknown = 0,
    //             UserInterrupted = 1,
    //             NoAnalyze = 8,
    //             NoBuild = 9,
    //             TimeOut = 2,
    //             RemoteEnvironmentFailure = 3,
    //             Internal = 4,
    //             LoadingFailure = 5,
    //             AnalysisFailure = 6,
    //             Skipped = 7,
    //             Incomplete = 10,
    //             OutOfMemory = 11,
    //         }
    //     });

    //     let file = syn::parse_file(output.unwrap().to_string().as_str()).unwrap();
    //     //eprintln!("{}", prettyplease::unparse(&file));
    //     assert!(false);
    // }

    // #[test]
    // fn try_enum_test() {
    //     let output = try_enumeration(quote! {
    //         #[derive(
    //             Clone,
    //             Copy,
    //             Debug,
    //             PartialEq,
    //             Eq,
    //             Hash,
    //             PartialOrd,
    //             Ord,
    //             ::prost::Enumeration
    //         )]
    //         #[repr(i32)]
    //         pub enum SymlinkBehavior {
    //             Copy = 1,
    //             Dereference = 2,
    //         }
    //     });

    //     let file = syn::parse_file(output.unwrap().to_string().as_str()).unwrap();
    //     eprintln!("{}", prettyplease::unparse(&file));
    //     assert!(false);
    // }

    #[test]
    fn try_message_test() {
        let output = try_message(quote! {
            pub struct TargetMetrics {
                /// DEPRECATED
                /// No longer populated. It never measured what it was supposed to (targets
                /// loaded): it counted targets that were analyzed even if the underlying
                /// package had not changed.
                /// TODO(janakr): rename and remove.
                #[deprecated]
                #[prost(int64, tag = "1")]
                #[starbuf(
                    path = "build_event_stream.BuildMetrics.TargetMetrics.targets_loaded",
                    return_expr = "::starlark::values::none::NoneOr::from_option(this.id.as_ref().map(|id| id.id.clone().unwrap()))",
                    return_type = "::starlark::values::none::NoneOr<build_event_id::Id>"
                )]
                pub targets_loaded: i64,
                /// Number of targets/aspects configured during this build. Does not include
                /// targets/aspects that were configured on prior builds on this server and
                /// were cached. See BuildGraphMetrics below if you need that.
                #[prost(int64, tag = "2")]
                #[starbuf(
                    path = "build_event_stream.BuildMetrics.TargetMetrics.targets_configured"
                )]
                pub targets_configured: i64,
                /// Number of configured targets analyzed during this build. Does not include
                /// aspects. Used mainly to allow consumers of targets_configured, which used
                /// to not include aspects, to normalize across the Blaze release that
                /// switched targets_configured to include aspects.
                #[prost(int64, tag = "3")]
                #[starbuf(
                    path = "build_event_stream.BuildMetrics.TargetMetrics.targets_configured_not_including_aspects"
                )]
                pub targets_configured_not_including_aspects: i64,
            }
        });
        let out = output.unwrap().to_string();

        let file = syn::parse_file(out.as_str()).unwrap();
        eprintln!("{}", prettyplease::unparse(&file));
        assert!(false);
    }

    // #[test]
    // fn try_message_test_complex() {
    //     let output = try_message(quote! {
    //         #[display("ActionGraphContainer")]
    //         #[derive(Clone, PartialEq, ::prost::Message)]
    //         pub struct ActionGraphContainer {
    //             #[prost(message, repeated, tag = "1")]
    //             pub artifacts: ::prost::alloc::vec::Vec<Artifact>,
    //             #[prost(message, repeated, tag = "2")]
    //             pub actions: ::prost::alloc::vec::Vec<Action>,
    //             #[prost(message, repeated, tag = "3")]
    //             pub targets: ::prost::alloc::vec::Vec<Target>,
    //             #[prost(message, repeated, tag = "4")]
    //             pub dep_set_of_files: ::prost::alloc::vec::Vec<DepSetOfFiles>,
    //             #[prost(message, repeated, tag = "5")]
    //             pub configuration: ::prost::alloc::vec::Vec<Configuration>,
    //             #[prost(message, repeated, tag = "6")]
    //             pub aspect_descriptors: ::prost::alloc::vec::Vec<AspectDescriptor>,
    //             #[prost(message, repeated, tag = "7")]
    //             pub rule_classes: ::prost::alloc::vec::Vec<RuleClass>,
    //             #[prost(message, repeated, tag = "8")]
    //             pub path_fragments: ::prost::alloc::vec::Vec<PathFragment>,
    //         }
    //     });
    //     let out = output.unwrap().to_string();
    //     eprintln!("{}", out);
    //     let file = syn::parse_file(out.as_str()).unwrap();
    //     eprintln!("{}", prettyplease::unparse(&file));
    //     assert!(false);
    // }

    // #[test]
    // fn try_message_test_complex() {
    //     let output = try_message(quote! {
    //         #[display("WorkerMetrics")]
    //         #[derive(Clone, PartialEq, ::prost::Message)]
    //         pub struct WorkerMetrics {
    //             #[deprecated]
    //             #[prost(int32, tag = "1")]
    //             pub worker_id: i32,
    //             #[prost(uint32, repeated, tag = "8")]
    //             pub worker_ids: ::prost::alloc::vec::Vec<u32>,
    //             #[prost(uint32, tag = "2")]
    //             pub process_id: u32,
    //             #[prost(string, tag = "3")]
    //             pub mnemonic: ::prost::alloc::string::String,
    //             #[prost(bool, tag = "4")]
    //             pub is_multiplex: bool,
    //             #[prost(bool, tag = "5")]
    //             pub is_sandbox: bool,
    //             #[prost(bool, tag = "6")]
    //             pub is_measurable: bool,
    //             #[prost(int64, tag = "9")]
    //             pub worker_key_hash: i64,
    //             #[prost(enumeration = "worker_metrics::WorkerStatus", tag = "10")]
    //             pub worker_status: i32,
    //             #[prost(
    //                 enumeration = "super::super::failure_details::worker::Code",
    //                 optional,
    //                 tag = "12"
    //             )]
    //             pub code: ::core::option::Option<i32>,
    //             #[prost(int64, tag = "11")]
    //             pub actions_executed: i64,
    //             #[prost(int64, tag = "13")]
    //             pub prior_actions_executed: i64,
    //             #[prost(message, repeated, tag = "7")]
    //             pub worker_stats: ::prost::alloc::vec::Vec<worker_metrics::WorkerStats>,
    //         }
    //     });
    //     let out = output.unwrap().to_string();
    //     eprintln!("{}", out);
    //     let file = syn::parse_file(out.as_str()).unwrap();
    //     eprintln!("{}", prettyplease::unparse(&file));
    //     assert!(false);
    // }

    // #[test]
    // fn try_message_test_complex() {
    //     let output = try_message(quote! {
    //         #[display("BuildMetadata")]
    //         #[derive(Clone, PartialEq, ::prost::Message)]
    //         pub struct BuildMetadata {
    //             #[prost(map = "string, string", tag = "1")]
    //             pub metadata: ::std::collections::HashMap<
    //                 ::prost::alloc::string::String,
    //                 ::prost::alloc::string::String,
    //             >,
    //         }
    //     });
    //     let out = output.unwrap().to_string();
    //     eprintln!("{}", out);
    //     let file = syn::parse_file(out.as_str()).unwrap();
    //     eprintln!("{}", prettyplease::unparse(&file));
    //     assert!(false);
    // }
}
