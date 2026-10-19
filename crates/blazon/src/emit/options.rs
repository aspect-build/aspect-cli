//! Printing the option schema: one static table per version.

use crate::options::{OptKind, OptionDef, Schema};
use proc_macro2::{Literal, TokenStream};
use quote::quote;

fn kind(k: &OptKind) -> TokenStream {
    match k {
        OptKind::Bool => quote!(Kind::Bool),
        OptKind::Int => quote!(Kind::Int),
        OptKind::Long => quote!(Kind::Long),
        OptKind::Double => quote!(Kind::Double),
        OptKind::String => quote!(Kind::String),
        OptKind::Label => quote!(Kind::Label),
        OptKind::Path => quote!(Kind::Path),
        OptKind::Duration => quote!(Kind::Duration),
        OptKind::Void => quote!(Kind::Void),
        OptKind::Enum(values) => quote!(Kind::Enum(&[#(#values),*])),
        OptKind::List(inner) => {
            let inner = kind(inner);
            quote!(Kind::List(&#inner))
        }
        OptKind::Dict(inner) => {
            let inner = kind(inner);
            quote!(Kind::Dict(&#inner))
        }
        OptKind::Opaque(converter) => quote!(Kind::Opaque(#converter)),
    }
}

fn def(o: &OptionDef) -> TokenStream {
    let OptionDef {
        name,
        class,
        field,
        default,
        help,
        value_help,
        category,
        allows_multiple,
        deprecated,
        deprecation_warning,
        expansion,
        fragment,
        ..
    } = o;
    let abbrev = Literal::character(o.abbrev.unwrap_or('\0'));
    let kind = kind(&o.kind);
    let effect_tags = &o.effect_tags;
    let metadata_tags = &o.metadata_tags;
    let expands_to = &o.expands_to;
    let implicit_requirements = &o.implicit_requirements;
    let old_names = &o.old_names;
    quote!(OptionDef {
        name: #name,
        abbrev: #abbrev,
        class: #class,
        field: #field,
        kind: #kind,
        default: #default,
        help: #help,
        value_help: #value_help,
        category: #category,
        effect_tags: &[#(#effect_tags),*],
        metadata_tags: &[#(#metadata_tags),*],
        allows_multiple: #allows_multiple,
        deprecated: #deprecated,
        deprecation_warning: #deprecation_warning,
        expansion: #expansion,
        expands_to: &[#(#expands_to),*],
        implicit_requirements: &[#(#implicit_requirements),*],
        old_names: &[#(#old_names),*],
        fragment: #fragment,
    })
}

/// The `options` module of a version's file.
pub fn module(schema: &Schema) -> TokenStream {
    let defs = schema.options.iter().map(def);
    quote!(
        /// Bazel's options for this release, selected by the spec's `[options]`.
        pub mod options {
            use blazon_rt::options::{Kind, OptionDef};

            /// The options, sorted by name.
            pub static OPTIONS: &[OptionDef] = &[#(#defs),*];

            /// The option called `name`, or that used to be.
            pub fn option(name: &str) -> Option<&'static OptionDef> {
                blazon_rt::options::find(OPTIONS, name)
            }

            /// Option names that look like `name`, nearest first.
            pub fn closest(name: &str) -> Vec<&'static str> {
                blazon_rt::options::closest(OPTIONS, name)
            }
        }
    )
}
