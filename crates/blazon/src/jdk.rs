//! The standard Java types `blazon-rt` provides as hand-written handles
//! (`blazon_rt::java`). The generator maps their descriptors to those types and
//! never generates them; a spec that lists one, or a selected signature that
//! names a JDK class missing here, is an error.

pub struct Mapped {
    pub internal: &'static str,
    /// The name the generated module re-exports the type under.
    pub rust: &'static str,
    /// The path in `blazon_rt::java`.
    pub path: &'static str,
    pub arity: usize,
}

const fn m(internal: &'static str, rust: &'static str, path: &'static str, arity: usize) -> Mapped {
    Mapped {
        internal,
        rust,
        path,
        arity,
    }
}

pub const MAPPED: &[Mapped] = &[
    m("java/lang/Object", "Object", "lang::Object", 0),
    m("java/lang/Class", "Class", "lang::Class", 1),
    m("java/lang/String", "JavaString", "lang::String", 0),
    m("java/lang/Boolean", "Boolean", "lang::Boolean", 0),
    m("java/lang/Integer", "Integer", "lang::Integer", 0),
    m("java/lang/Long", "Long", "lang::Long", 0),
    m("java/lang/Iterable", "Iterable", "lang::Iterable", 1),
    m("java/lang/Runnable", "Runnable", "lang::Runnable", 0),
    m("java/lang/Throwable", "Throwable", "lang::Throwable", 0),
    m("java/lang/Exception", "Exception", "lang::Exception", 0),
    m(
        "java/lang/RuntimeException",
        "RuntimeException",
        "lang::RuntimeException",
        0,
    ),
    m("java/util/Iterator", "JavaIterator", "util::Iterator", 1),
    m("java/util/Collection", "Collection", "util::Collection", 1),
    m("java/util/List", "List", "util::List", 1),
    m("java/util/Set", "Set", "util::Set", 1),
    m("java/util/Map", "Map", "util::Map", 2),
    m("java/util/SortedMap", "SortedMap", "util::SortedMap", 2),
    m("java/util/Map$Entry", "Entry", "util::Entry", 2),
    m("java/util/Optional", "Optional", "util::Optional", 1),
    m("java/util/UUID", "UUID", "util::UUID", 0),
    m(
        "java/io/OutputStream",
        "OutputStream",
        "io::OutputStream",
        0,
    ),
    m(
        "java/io/ByteArrayOutputStream",
        "ByteArrayOutputStream",
        "io::ByteArrayOutputStream",
        0,
    ),
    m(
        "java/io/BufferedOutputStream",
        "BufferedOutputStream",
        "io::BufferedOutputStream",
        0,
    ),
    m("java/math/BigInteger", "BigInteger", "math::BigInteger", 0),
    m("java/time/Instant", "Instant", "time::Instant", 0),
    m("java/time/Duration", "Duration", "time::Duration", 0),
];

/// The mapped supertypes of a mapped class, which keep the class's type
/// arguments: a `List<E>` is a `Collection<E>` and an `Iterable<E>`.
pub fn mapped_supers(internal: &str) -> &'static [&'static str] {
    match internal {
        "java/util/List" | "java/util/Set" => &["java/util/Collection", "java/lang/Iterable"],
        "java/util/Collection" => &["java/lang/Iterable"],
        "java/util/SortedMap" => &["java/util/Map"],
        "java/lang/Exception" => &["java/lang/Throwable"],
        "java/lang/RuntimeException" => &["java/lang/Exception", "java/lang/Throwable"],
        "java/io/ByteArrayOutputStream" | "java/io/BufferedOutputStream" => {
            &["java/io/OutputStream"]
        }
        _ => &[],
    }
}

pub fn lookup(internal: &str) -> Option<&'static Mapped> {
    MAPPED.iter().find(|m| m.internal == internal)
}

/// A class that lives in the JDK, mapped or not.
pub fn is_jdk(internal: &str) -> bool {
    internal.starts_with("java/") || internal.starts_with("javax/")
}

/// The `pub use` lines for a generated module.
pub fn reexports() -> proc_macro2::TokenStream {
    let lines = MAPPED.iter().map(|m| {
        let path: syn::Path = syn::parse_str(&format!("blazon_rt::java::{}", m.path))
            .unwrap_or_else(|_| syn::parse_quote!(blazon_rt::java));
        let last = m.path.rsplit("::").next().unwrap_or(m.path);
        if last == m.rust {
            quote::quote!(pub use #path;)
        } else {
            let alias = syn::Ident::new(m.rust, proc_macro2::Span::call_site());
            quote::quote!(pub use #path as #alias;)
        }
    });
    quote::quote!(#(#lines)*)
}
