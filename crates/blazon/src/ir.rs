//! What a spec resolves to against one Bazel version: the classes to bind,
//! the members selected, their signatures with generics and nullability, and
//! the Starlark facets. Everything after this is printing.

use crate::classfile::Constant;
use crate::signature::Ty;
use std::collections::BTreeSet;

/// Whether a member belongs to the instance or to the class.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Target {
    Instance,
    Static,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MemberKind {
    Method(Target),
    Constructor,
    Field(Target),
}

impl MemberKind {
    pub fn target(self) -> Target {
        match self {
            MemberKind::Method(t) | MemberKind::Field(t) => t,
            MemberKind::Constructor => Target::Static,
        }
    }

    pub fn is_field(self) -> bool {
        matches!(self, MemberKind::Field(_))
    }
}

#[derive(Clone, Debug)]
pub struct ParamSig {
    pub ty: Ty,
    pub nullable: bool,
}

/// What a member takes and gives, with generics from the class file.
#[derive(Clone, Debug)]
pub struct Signature {
    pub params: Vec<ParamSig>,
    pub ret: Ty,
    pub ret_nullable: bool,
    /// Method-level type variables, which erase to `Object`.
    pub type_params: Vec<String>,
}

#[derive(Clone, Debug)]
pub struct Member {
    pub kind: MemberKind,
    pub java_name: String,
    pub rust_name: String,
    /// The JNI descriptor, what the call resolves against.
    pub descriptor: String,
    pub public: bool,
    /// `public static final`, for the generated doc comment.
    pub modifiers: String,
    pub sig: Signature,
    /// The selector text from the spec (or `@starlark name`), which lines members
    /// up across versions.
    pub selector: String,
    /// The value of a `static final` primitive or `String` constant.
    pub constant: Option<Constant>,
    /// The `@StarlarkMethod` doc of the method, as plain text.
    pub doc: Option<String>,
    /// Exposed to Starlark as an attribute (spec `@`), for a class without annotations.
    pub attribute: bool,
    /// The supertype that declares the member, when it is not this class (internal name).
    pub declared_in: Option<String>,
}

/// A supertype bound in the same module (or provided by `blazon-rt`), with
/// the type arguments it has as a supertype of the class.
#[derive(Clone, Debug)]
pub struct SuperType {
    pub internal: String,
    pub args: Vec<Ty>,
}

#[derive(Clone, Debug)]
pub struct Class {
    pub java: String,
    pub internal: String,
    pub rust: String,
    pub type_params: Vec<String>,
    pub is_interface: bool,
    pub supers: Vec<SuperType>,
    /// Implements `java.lang.Comparable`, so the handle is `Ord`.
    pub comparable: bool,
    pub members: Vec<Member>,
    /// Selectors that matched nothing in this version.
    pub missing: Vec<String>,
    /// The Starlark value type bound for this class, when there is one.
    pub starlark: Option<Facet>,
}

/// One parameter of a Starlark-visible method.
#[derive(Clone, Debug)]
pub struct FacetParam {
    pub name: String,
    /// Java types the Starlark argument may be, empty for the Java parameter type.
    pub allowed: Vec<Ty>,
    /// The Starlark default expression; `None` when required.
    pub default: Option<String>,
    pub named: bool,
    pub positional: bool,
}

/// A method or attribute of a Starlark value.
#[derive(Clone, Debug)]
pub struct FacetItem {
    pub name: String,
    pub doc: String,
    /// Index into `Class::members`: the generated call this item makes.
    pub member: usize,
    /// A `structField`, or a Java field: read as `x.name`, not called.
    pub attribute: bool,
    pub params: Vec<FacetParam>,
    pub use_thread: bool,
    pub use_semantics: bool,
}

#[derive(Clone, Debug, Default)]
pub struct Facet {
    /// `@StarlarkBuiltin` name, else the Java simple name.
    pub type_name: String,
    pub doc: String,
    /// Bound only because a selected signature names it; no methods.
    pub opaque: bool,
    pub interfaces: BTreeSet<String>,
    pub ancestors: BTreeSet<String>,
    pub items: Vec<FacetItem>,
    /// Types named only by `generic1` of an allowed type, which must be bound.
    pub refs: Vec<(Ty, String)>,
    /// Annotated methods left out, with the reason.
    pub skipped: Vec<(String, String)>,
    /// (selector, shape) pairs for the shape report.
    pub shapes: Vec<(String, String)>,
}
