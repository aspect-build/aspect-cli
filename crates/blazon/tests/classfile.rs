//! What the class-file reader and the annotation model make of the fixture
//! (`tests/fixture/src`).

use blazon::classfile::{Classes, Constant};
use blazon::jar::Jar;
use blazon::signature::{Arg, Ty};
use std::path::PathBuf;

fn classes() -> Classes {
    let jar = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixture/fixture.jar");
    Classes::new(Jar::open(&jar).expect("fixture jar"))
}

#[test]
fn generics_and_supertypes() {
    let mut c = classes();
    let w = c.require("fx/Widget", "test").expect("class");
    assert_eq!(
        w.type_params
            .iter()
            .map(|t| t.name.as_str())
            .collect::<Vec<_>>(),
        ["K", "V"]
    );
    assert_eq!(
        w.super_class,
        Some(Ty::Class {
            name: "fx/Base".into(),
            args: vec![Arg::Exact(Ty::Var("K".into()))]
        })
    );
    let names: Vec<String> = w
        .interfaces
        .iter()
        .filter_map(|t| match t {
            Ty::Class { name, .. } => Some(name.clone()),
            _ => None,
        })
        .collect();
    assert!(names.contains(&"fx/Shape".to_string()));
    assert!(names.contains(&"java/lang/Comparable".to_string()));
    let builtin = w.builtin.as_ref().expect("@StarlarkBuiltin");
    assert_eq!(builtin.name, "widget");
}

#[test]
fn nullability_comes_from_annotations() {
    let mut c = classes();
    let w = c.require("fx/Widget", "test").expect("class");
    let find = w.methods.iter().find(|m| m.name == "find").expect("find");
    assert!(find.nullable_ret, "@Nullable on the method");
    assert_eq!(
        find.nullable_params,
        [false, true],
        "@Nullable on the second parameter"
    );
    let join = w.methods.iter().find(|m| m.name == "join").expect("join");
    assert!(!join.nullable_ret);
    assert!(join.access.varargs);
}

#[test]
fn constants_and_access() {
    let mut c = classes();
    let w = c.require("fx/Widget", "test").expect("class");
    let max = w.fields.iter().find(|f| f.name == "MAX").expect("MAX");
    assert_eq!(max.constant, Some(Constant::Int(7)));
    let name = w.fields.iter().find(|f| f.name == "NAME").expect("NAME");
    assert_eq!(name.constant, Some(Constant::Str("widget".into())));
    let big = w.fields.iter().find(|f| f.name == "BIG").expect("BIG");
    assert_eq!(big.constant, Some(Constant::Long(1 << 40)));
    let secret = w
        .fields
        .iter()
        .find(|f| f.name == "secret")
        .expect("secret");
    assert!(secret.access.private && !secret.access.public);
    let count = w.fields.iter().find(|f| f.name == "count").expect("count");
    assert!(count.access.is_final);
    let hidden = w
        .methods
        .iter()
        .find(|m| m.name == "hidden")
        .expect("hidden");
    assert!(hidden.access.private);
}

#[test]
fn starlark_annotations() {
    let mut c = classes();
    let w = c.require("fx/Widget", "test").expect("class");
    let size = w
        .methods
        .iter()
        .find(|m| m.name == "size")
        .and_then(|m| m.starlark.clone())
        .expect("size");
    assert!(size.struct_field);
    assert_eq!(size.name, "size");
    let scaled = w
        .methods
        .iter()
        .find(|m| m.name == "scaled")
        .and_then(|m| m.starlark.clone())
        .expect("scaled");
    assert!(scaled.allow_return_nones);
    assert_eq!(scaled.params.len(), 2);
    assert_eq!(scaled.params[0].name, "factor");
    assert_eq!(scaled.params[0].default.as_deref(), Some("1"));
    assert_eq!(scaled.params[0].allowed.len(), 2);
    assert!(scaled.params[0].positional && !scaled.params[0].named);
    assert_eq!(scaled.params[1].default.as_deref(), Some("None"));
    assert!(scaled.params[1].named && !scaled.params[1].positional);
}

#[test]
fn missing_classes_are_none() {
    let mut c = classes();
    assert!(c.get("fx/Nope").expect("lookup").is_none());
    assert!(c.require("fx/Nope", "the spec").is_err());
}
