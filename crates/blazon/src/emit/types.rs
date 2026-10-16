//! Java types as the Rust types of the generated API: what a parameter takes,
//! what a result gives, and how the call converts between them.

use crate::jdk;
use crate::signature::{Arg, Ty};
use std::collections::BTreeMap;
use std::fmt;

/// A class bound in the module, or mapped to `blazon_rt::java`.
pub struct Bound {
    pub rust: String,
    pub arity: usize,
}

pub fn bound_map<'a>(
    classes: impl Iterator<Item = (&'a str, &'a str, usize)>,
) -> BTreeMap<String, Bound> {
    let mut bound: BTreeMap<String, Bound> = BTreeMap::new();
    for m in jdk::MAPPED {
        bound.insert(
            m.internal.into(),
            Bound {
                rust: m.rust.into(),
                arity: m.arity,
            },
        );
    }
    for (internal, rust, arity) in classes {
        bound.insert(
            internal.into(),
            Bound {
                rust: rust.into(),
                arity,
            },
        );
    }
    bound
}

/// The scope a member's types are read in: the module's classes, and the type
/// variables of the class the member is in.
pub struct Scope<'a> {
    pub bound: &'a BTreeMap<String, Bound>,
    pub class_vars: &'a [String],
}

/// Why a type cannot be expressed.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Problem {
    /// A class that is not bound: add it to the spec.
    Unbound(String),
    /// A JDK class that `blazon-rt` does not provide.
    MissingJdk(String),
    /// A form with no Rust spelling (`int[]`, a multi-dimensional array).
    Unsupported(String),
}

impl fmt::Display for Problem {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Problem::Unbound(c) => write!(f, "{} is not bound", c.replace('/', ".")),
            Problem::MissingJdk(c) => {
                write!(
                    f,
                    "{} is a JDK class that blazon-rt does not provide",
                    c.replace('/', ".")
                )
            }
            Problem::Unsupported(s) => write!(f, "{s} cannot be expressed"),
        }
    }
}

/// A reference type as a generated handle type.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Ref {
    Handle {
        rust: String,
        args: Vec<Ref>,
    },
    /// A type variable of the class.
    Var(String),
    /// `?` or `? super X` in a parameter: the member is generic over it, so the
    /// argument may be any instantiation. Numbered per member by the printer.
    Wild(usize),
}

impl Ref {
    /// The reference with wildcards as `Object`, for code that cannot name them.
    pub fn concrete(&self) -> Ref {
        match self {
            Ref::Handle { rust, args } => Ref::Handle {
                rust: rust.clone(),
                args: args.iter().map(Ref::concrete).collect(),
            },
            Ref::Wild(_) => Ref::object(),
            other => other.clone(),
        }
    }

    /// Numbers the wildcards from `*next`, in order.
    pub fn number_wildcards(&mut self, next: &mut usize) {
        match self {
            Ref::Handle { args, .. } => args.iter_mut().for_each(|a| a.number_wildcards(next)),
            Ref::Wild(n) => {
                *n = *next;
                *next += 1;
            }
            Ref::Var(_) => {}
        }
    }

    pub fn object() -> Ref {
        Ref::Handle {
            rust: "Object".into(),
            args: Vec::new(),
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Prim {
    Boolean,
    Byte,
    Char,
    Short,
    Int,
    Long,
    Float,
    Double,
}

impl Prim {
    pub fn of(c: char) -> Option<Prim> {
        Some(match c {
            'Z' => Prim::Boolean,
            'B' => Prim::Byte,
            'C' => Prim::Char,
            'S' => Prim::Short,
            'I' => Prim::Int,
            'J' => Prim::Long,
            'F' => Prim::Float,
            'D' => Prim::Double,
            _ => return None,
        })
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Boxed {
    Bool,
    Int,
    Long,
}

/// What a parameter is in Rust.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ParamKind {
    Prim(Prim),
    Str,
    Boxed(Boxed),
    Handle(Ref),
    Optional(Ref),
    Bytes,
    StrArray,
    /// A reference array, with the element class in internal form.
    ObjArray {
        element: Ref,
        class: String,
    },
}

/// What a result is in Rust.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum RetKind {
    Void,
    Prim(Prim),
    Str,
    Boxed(Boxed),
    Handle(Ref),
    Optional(Ref),
    Bytes,
    StrArray,
    ObjArray(Ref),
}

impl Scope<'_> {
    /// A reference type as a handle type; `top` is whether it is the whole
    /// type of a parameter or result, where an unbound class is a problem
    /// rather than `Object`.
    pub fn reference(&self, t: &Ty, top: bool) -> Result<Ref, Problem> {
        self.reference_in(t, top, false)
    }

    fn reference_in(&self, t: &Ty, top: bool, wild: bool) -> Result<Ref, Problem> {
        match t {
            Ty::Var(v) if self.class_vars.contains(v) => Ok(Ref::Var(v.clone())),
            Ty::Var(_) => Ok(Ref::object()),
            Ty::Class { name, args } => match self.bound.get(name) {
                Some(b) => {
                    let args = if b.arity > 0 && args.len() == b.arity {
                        args.iter()
                            .map(|a| self.type_arg(a, wild))
                            .collect::<Result<_, _>>()?
                    } else {
                        Vec::new()
                    };
                    Ok(Ref::Handle {
                        rust: b.rust.clone(),
                        args,
                    })
                }
                None if top && jdk::is_jdk(name) => Err(Problem::MissingJdk(name.clone())),
                None if top => Err(Problem::Unbound(name.clone())),
                None => Ok(Ref::object()),
            },
            Ty::Array(_) => Ok(Ref::object()),
            Ty::Prim(_) | Ty::Void => Err(Problem::Unsupported(
                "a primitive as a type argument".into(),
            )),
        }
    }

    /// A type argument of a supertype, as a handle type.
    pub fn reference_arg(&self, t: &Ty) -> Option<Ref> {
        self.reference(t, false).ok()
    }

    fn type_arg(&self, a: &Arg, wild: bool) -> Result<Ref, Problem> {
        match a {
            Arg::Exact(t) | Arg::Extends(t) => self.reference_in(t, false, wild),
            Arg::Any | Arg::Super(_) if wild => Ok(Ref::Wild(0)),
            Arg::Any | Arg::Super(_) => Ok(Ref::object()),
        }
    }

    fn element(&self, t: &Ty) -> Result<(Ref, String), Problem> {
        let class = match t {
            Ty::Class { name, .. } => name.clone(),
            _ => "java/lang/Object".into(),
        };
        Ok((self.reference(t, true)?, class))
    }

    pub fn param(&self, t: &Ty) -> Result<ParamKind, Problem> {
        Ok(match t {
            Ty::Prim(c) => ParamKind::Prim(
                Prim::of(*c).ok_or_else(|| Problem::Unsupported(format!("primitive {c}")))?,
            ),
            Ty::Void => return Err(Problem::Unsupported("a void parameter".into())),
            Ty::Class { name, args } => match name.as_str() {
                "java/lang/String" => ParamKind::Str,
                "java/lang/Boolean" => ParamKind::Boxed(Boxed::Bool),
                "java/lang/Integer" => ParamKind::Boxed(Boxed::Int),
                "java/lang/Long" => ParamKind::Boxed(Boxed::Long),
                "java/util/Optional" => ParamKind::Optional(match args.first() {
                    Some(a) => self.type_arg(a, true)?,
                    None => Ref::object(),
                }),
                _ => ParamKind::Handle(self.reference_in(t, true, true)?),
            },
            Ty::Var(_) => ParamKind::Handle(self.reference_in(t, true, true)?),
            Ty::Array(inner) => match &**inner {
                Ty::Prim('B') => ParamKind::Bytes,
                Ty::Class { name, .. } if name == "java/lang/String" => ParamKind::StrArray,
                Ty::Class { .. } | Ty::Var(_) => {
                    let (element, class) = self.element(inner)?;
                    ParamKind::ObjArray { element, class }
                }
                other => {
                    return Err(Problem::Unsupported(format!(
                        "an array of {}",
                        crate::signature::pretty(other)
                    )));
                }
            },
        })
    }

    pub fn result(&self, t: &Ty) -> Result<RetKind, Problem> {
        Ok(match t {
            Ty::Void => RetKind::Void,
            Ty::Prim(c) => RetKind::Prim(
                Prim::of(*c).ok_or_else(|| Problem::Unsupported(format!("primitive {c}")))?,
            ),
            Ty::Class { name, args } => match name.as_str() {
                "java/lang/String" => RetKind::Str,
                "java/lang/Boolean" => RetKind::Boxed(Boxed::Bool),
                "java/lang/Integer" => RetKind::Boxed(Boxed::Int),
                "java/lang/Long" => RetKind::Boxed(Boxed::Long),
                "java/util/Optional" => RetKind::Optional(match args.first() {
                    Some(a) => self.type_arg(a, false)?,
                    None => Ref::object(),
                }),
                _ => RetKind::Handle(self.reference(t, true)?),
            },
            Ty::Var(_) => RetKind::Handle(self.reference(t, true)?),
            Ty::Array(inner) => match &**inner {
                Ty::Prim('B') => RetKind::Bytes,
                Ty::Class { name, .. } if name == "java/lang/String" => RetKind::StrArray,
                Ty::Class { .. } | Ty::Var(_) => RetKind::ObjArray(self.reference(inner, true)?),
                other => {
                    return Err(Problem::Unsupported(format!(
                        "an array of {}",
                        crate::signature::pretty(other)
                    )));
                }
            },
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn class(name: &str, args: Vec<Arg>) -> Ty {
        Ty::Class {
            name: name.into(),
            args,
        }
    }

    fn scope<'a>(bound: &'a BTreeMap<String, Bound>, vars: &'a [String]) -> Scope<'a> {
        Scope {
            bound,
            class_vars: vars,
        }
    }

    #[test]
    fn scalars_and_strings() {
        let bound = bound_map(std::iter::empty());
        let s = scope(&bound, &[]);
        assert_eq!(s.param(&Ty::Prim('I')), Ok(ParamKind::Prim(Prim::Int)));
        assert_eq!(
            s.param(&class("java/lang/String", vec![])),
            Ok(ParamKind::Str)
        );
        assert_eq!(
            s.result(&class("java/lang/Long", vec![])),
            Ok(RetKind::Boxed(Boxed::Long))
        );
        assert_eq!(s.result(&Ty::Void), Ok(RetKind::Void));
    }

    #[test]
    fn generics_collapse_wildcards() {
        let bound = bound_map(std::iter::empty());
        let s = scope(&bound, &[]);
        let list = class(
            "java/util/List",
            vec![Arg::Extends(class("java/lang/String", vec![]))],
        );
        assert_eq!(
            s.param(&list),
            Ok(ParamKind::Handle(Ref::Handle {
                rust: "List".into(),
                args: vec![Ref::Handle {
                    rust: "JavaString".into(),
                    args: vec![]
                }]
            }))
        );
        let any = class("java/util/List", vec![Arg::Any]);
        assert_eq!(
            s.result(&any),
            Ok(RetKind::Handle(Ref::Handle {
                rust: "List".into(),
                args: vec![Ref::object()]
            }))
        );
    }

    #[test]
    fn unbound_and_jdk_problems() {
        let bound = bound_map(std::iter::empty());
        let s = scope(&bound, &[]);
        assert_eq!(
            s.param(&class("com/x/Missing", vec![])),
            Err(Problem::Unbound("com/x/Missing".into()))
        );
        assert_eq!(
            s.result(&class("java/util/concurrent/Phaser", vec![])),
            Err(Problem::MissingJdk("java/util/concurrent/Phaser".into()))
        );
        let nested = class(
            "java/util/List",
            vec![Arg::Exact(class("com/x/Missing", vec![]))],
        );
        assert!(
            s.param(&nested).is_ok(),
            "an unbound type argument is Object"
        );
    }

    #[test]
    fn wildcards_in_parameters_are_generic() {
        let bound = bound_map(std::iter::empty());
        let s = scope(&bound, &[]);
        let any = class("java/util/List", vec![Arg::Any]);
        match s.param(&any) {
            Ok(ParamKind::Handle(Ref::Handle { args, .. })) => assert_eq!(args, [Ref::Wild(0)]),
            other => panic!("{other:?}"),
        }
        let mut r = Ref::Handle {
            rust: "Map".into(),
            args: vec![Ref::Wild(0), Ref::object(), Ref::Wild(0)],
        };
        let mut next = 0;
        r.number_wildcards(&mut next);
        assert_eq!(next, 2);
        assert_eq!(
            r.concrete(),
            Ref::Handle {
                rust: "Map".into(),
                args: vec![Ref::object(); 3]
            }
        );
    }

    #[test]
    fn arrays() {
        let bound = bound_map(std::iter::empty());
        let s = scope(&bound, &[]);
        let bytes = Ty::Array(Box::new(Ty::Prim('B')));
        assert_eq!(s.param(&bytes), Ok(ParamKind::Bytes));
        let ints = Ty::Array(Box::new(Ty::Prim('I')));
        assert!(matches!(s.param(&ints), Err(Problem::Unsupported(_))));
        let strings = Ty::Array(Box::new(class("java/lang/String", vec![])));
        assert_eq!(s.result(&strings), Ok(RetKind::StrArray));
    }

    #[test]
    fn class_variables() {
        let bound = bound_map(std::iter::empty());
        let vars = vec!["T".to_string()];
        let s = scope(&bound, &vars);
        assert_eq!(
            s.result(&Ty::Var("T".into())),
            Ok(RetKind::Handle(Ref::Var("T".into())))
        );
        assert_eq!(
            s.result(&Ty::Var("U".into())),
            Ok(RetKind::Handle(Ref::object()))
        );
    }
}
