//! JVM descriptors and generic `Signature` attributes, parsed to a tree.
//! Descriptors are the generic-free subset, so one parser serves both.

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Ty {
    Prim(char),
    Void,
    Class { name: String, args: Vec<Arg> },
    Array(Box<Ty>),
    Var(String),
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Arg {
    Any,
    Extends(Ty),
    Super(Ty),
    Exact(Ty),
}

pub struct MethodTy {
    pub type_params: Vec<String>,
    pub params: Vec<Ty>,
    pub ret: Ty,
}

/// A type parameter and its bounds, `<K::Ljava/lang/Object;>`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TypeParam {
    pub name: String,
    pub bounds: Vec<Ty>,
}

/// What a class `Signature` says: its type parameters and generic supertypes.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ClassTy {
    pub type_params: Vec<TypeParam>,
    pub super_class: Ty,
    pub interfaces: Vec<Ty>,
}

pub fn parse_class(s: &str) -> Option<ClassTy> {
    let mut p = Parser {
        s: s.as_bytes(),
        i: 0,
    };
    let type_params = if p.peek() == Some(b'<') {
        p.type_params()?
    } else {
        Vec::new()
    };
    let super_class = p.ty()?;
    let mut interfaces = Vec::new();
    while p.peek().is_some() {
        interfaces.push(p.ty()?);
    }
    Some(ClassTy {
        type_params,
        super_class,
        interfaces,
    })
}

pub fn parse_type(s: &str) -> Option<Ty> {
    let mut p = Parser {
        s: s.as_bytes(),
        i: 0,
    };
    let t = p.ty()?;
    (p.i == p.s.len()).then_some(t)
}

pub fn parse_method(s: &str) -> Option<MethodTy> {
    let mut p = Parser {
        s: s.as_bytes(),
        i: 0,
    };
    let type_params = if p.peek() == Some(b'<') {
        p.type_params()?.into_iter().map(|t| t.name).collect()
    } else {
        Vec::new()
    };
    if p.next() != Some(b'(') {
        return None;
    }
    let mut params = Vec::new();
    while p.peek() != Some(b')') {
        params.push(p.ty()?);
    }
    p.next();
    let ret = p.ty()?;
    Some(MethodTy {
        type_params,
        params,
        ret,
    })
}

/// The type parameter names of a class `Signature`, e.g. `["K", "V"]`.
pub fn class_type_params(s: &str) -> Vec<String> {
    let mut p = Parser {
        s: s.as_bytes(),
        i: 0,
    };
    if p.peek() == Some(b'<') {
        p.type_params()
            .unwrap_or_default()
            .into_iter()
            .map(|t| t.name)
            .collect()
    } else {
        Vec::new()
    }
}

struct Parser<'a> {
    s: &'a [u8],
    i: usize,
}

impl Parser<'_> {
    fn peek(&self) -> Option<u8> {
        self.s.get(self.i).copied()
    }

    fn next(&mut self) -> Option<u8> {
        let c = self.peek()?;
        self.i += 1;
        Some(c)
    }

    fn until(&mut self, stops: &[u8]) -> String {
        let start = self.i;
        while let Some(c) = self.peek() {
            if stops.contains(&c) {
                break;
            }
            self.i += 1;
        }
        String::from_utf8_lossy(&self.s[start..self.i]).into_owned()
    }

    fn type_params(&mut self) -> Option<Vec<TypeParam>> {
        self.next();
        let mut out = Vec::new();
        while self.peek() != Some(b'>') {
            let name = self.until(b":");
            let mut bounds = Vec::new();
            while self.peek() == Some(b':') {
                self.next();
                if self.peek() == Some(b':') {
                    continue;
                }
                bounds.push(self.ty()?);
            }
            out.push(TypeParam { name, bounds });
        }
        self.next();
        Some(out)
    }

    fn ty(&mut self) -> Option<Ty> {
        Some(match self.next()? {
            c @ (b'B' | b'C' | b'D' | b'F' | b'I' | b'J' | b'S' | b'Z') => Ty::Prim(c as char),
            b'V' => Ty::Void,
            b'[' => Ty::Array(Box::new(self.ty()?)),
            b'T' => {
                let name = self.until(b";");
                self.next();
                Ty::Var(name)
            }
            b'L' => {
                let mut name = String::new();
                let mut args = Vec::new();
                loop {
                    name.push_str(&self.until(b"<;."));
                    match self.next()? {
                        b'<' => {
                            args.clear();
                            while self.peek() != Some(b'>') {
                                args.push(self.arg()?);
                            }
                            self.next();
                            match self.next()? {
                                b';' => break,
                                b'.' => name.push('$'),
                                _ => return None,
                            }
                        }
                        b';' => break,
                        b'.' => name.push('$'),
                        _ => return None,
                    }
                }
                Ty::Class { name, args }
            }
            _ => return None,
        })
    }

    fn arg(&mut self) -> Option<Arg> {
        Some(match self.peek()? {
            b'*' => {
                self.next();
                Arg::Any
            }
            b'+' => {
                self.next();
                Arg::Extends(self.ty()?)
            }
            b'-' => {
                self.next();
                Arg::Super(self.ty()?)
            }
            _ => Arg::Exact(self.ty()?),
        })
    }
}

impl Ty {
    /// Every type variable as `Object`.
    pub fn erase_vars(&self) -> Ty {
        match self {
            Ty::Var(_) => Ty::Class {
                name: "java/lang/Object".into(),
                args: Vec::new(),
            },
            Ty::Class { name, args } => Ty::Class {
                name: name.clone(),
                args: args
                    .iter()
                    .map(|a| match a {
                        Arg::Exact(t) => Arg::Exact(t.erase_vars()),
                        Arg::Extends(t) => Arg::Extends(t.erase_vars()),
                        Arg::Super(t) => Arg::Super(t.erase_vars()),
                        Arg::Any => Arg::Any,
                    })
                    .collect(),
            },
            Ty::Array(inner) => Ty::Array(Box::new(inner.erase_vars())),
            other => other.clone(),
        }
    }
}

pub fn pretty(t: &Ty) -> String {
    match t {
        Ty::Prim(c) => match c {
            'B' => "byte",
            'C' => "char",
            'D' => "double",
            'F' => "float",
            'I' => "int",
            'J' => "long",
            'S' => "short",
            _ => "boolean",
        }
        .into(),
        Ty::Void => "void".into(),
        Ty::Array(inner) => format!("{}[]", pretty(inner)),
        Ty::Var(v) => v.clone(),
        Ty::Class { name, args } => {
            let simple = name.rsplit('/').next().unwrap_or(name);
            if args.is_empty() {
                simple.to_string()
            } else {
                let args: Vec<String> = args
                    .iter()
                    .map(|a| match a {
                        Arg::Any => "?".to_string(),
                        Arg::Extends(t) => format!("? extends {}", pretty(t)),
                        Arg::Super(t) => format!("? super {}", pretty(t)),
                        Arg::Exact(t) => pretty(t),
                    })
                    .collect();
                format!("{simple}<{}>", args.join(", "))
            }
        }
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

    #[test]
    fn descriptors() {
        let m = parse_method("(ILjava/lang/String;[B)V").expect("method");
        assert_eq!(m.params.len(), 3);
        assert_eq!(m.params[0], Ty::Prim('I'));
        assert_eq!(m.params[2], Ty::Array(Box::new(Ty::Prim('B'))));
        assert_eq!(m.ret, Ty::Void);
        assert!(parse_method("(I").is_none());
    }

    #[test]
    fn generic_methods_with_wildcards() {
        let sig = "<T:Ljava/lang/Object;>(Ljava/util/List<+TT;>;Ljava/util/Map<Ljava/lang/String;-Ljava/lang/Integer;>;Ljava/util/List<*>;)TT;";
        let m = parse_method(sig).expect("method");
        assert_eq!(m.type_params, ["T"]);
        assert_eq!(
            m.params[0],
            class("java/util/List", vec![Arg::Extends(Ty::Var("T".into()))])
        );
        assert_eq!(
            m.params[1],
            class(
                "java/util/Map",
                vec![
                    Arg::Exact(class("java/lang/String", vec![])),
                    Arg::Super(class("java/lang/Integer", vec![]))
                ]
            )
        );
        assert_eq!(m.params[2], class("java/util/List", vec![Arg::Any]));
        assert_eq!(m.ret, Ty::Var("T".into()));
    }

    #[test]
    fn inner_classes_of_generic_classes() {
        let t = parse_type("Lfx/Outer<TK;>.Inner<TV;>;").expect("type");
        assert_eq!(
            t,
            class("fx/Outer$Inner", vec![Arg::Exact(Ty::Var("V".into()))])
        );
    }

    #[test]
    fn class_signatures() {
        let c = parse_class(
            "<K:Ljava/lang/Object;V::Ljava/lang/Comparable<TV;>;>Lfx/Base<TK;>;Lfx/Shape<TV;>;Ljava/lang/Comparable<Lfx/Widget<TK;TV;>;>;",
        )
        .expect("class");
        assert_eq!(c.type_params.len(), 2);
        assert_eq!(c.type_params[0].name, "K");
        assert_eq!(c.type_params[1].name, "V");
        assert_eq!(
            c.type_params[1].bounds,
            [class(
                "java/lang/Comparable",
                vec![Arg::Exact(Ty::Var("V".into()))]
            )]
        );
        assert_eq!(
            c.super_class,
            class("fx/Base", vec![Arg::Exact(Ty::Var("K".into()))])
        );
        assert_eq!(c.interfaces.len(), 2);
    }

    #[test]
    fn throws_clauses_are_ignored() {
        let m = parse_method("()V^Ljava/io/IOException;").expect("method");
        assert_eq!(m.ret, Ty::Void);
    }

    #[test]
    fn pretty_prints_java() {
        let t = class(
            "java/util/Map",
            vec![Arg::Exact(class("java/lang/String", vec![])), Arg::Any],
        );
        assert_eq!(pretty(&t), "Map<String, ?>");
        assert_eq!(pretty(&Ty::Array(Box::new(Ty::Prim('I')))), "int[]");
    }

    #[test]
    fn erasing_variables() {
        let t = class("java/util/List", vec![Arg::Exact(Ty::Var("E".into()))]);
        assert_eq!(
            t.erase_vars(),
            class(
                "java/util/List",
                vec![Arg::Exact(class("java/lang/Object", vec![]))]
            )
        );
    }
}
