//! The fixture's generated API and Starlark types, run in a JVM. One test: a
//! process has one JVM.

use blazon_rt::java::lang::Object;
use blazon_rt::java::util::List;
use blazon_rt::{Jvm, Result};
use blazon_selftest::fixture::vfixture as fx;
use starlark::environment::{Globals, Module};
use starlark::eval::Evaluator;
use starlark::syntax::{AstModule, Dialect};
use starlark::values::{Heap, Value};

fn eval(src: &str, name: &str, value: impl for<'v> Fn(Heap<'v>) -> Value<'v>) -> String {
    let ast = AstModule::parse("t.star", src.to_string(), &Dialect::Extended).expect("parse");
    Module::with_temp_heap(|module| {
        module.set(name, value(module.heap()));
        let mut eval = Evaluator::new(&module);
        Ok::<_, starlark::Error>(match eval.eval_module(ast, &Globals::standard()) {
            Ok(v) => v.to_string(),
            Err(e) => format!("ERR {e:#}"),
        })
    })
    .expect("heap")
}

#[test]
fn widget() -> Result<()> {
    let Some(libjvm) = blazon_selftest::libjvm() else {
        eprintln!("skipped: no libjvm");
        return Ok(());
    };
    let jar = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../blazon/tests/fixture/fixture.jar");
    let _jvm = Jvm::start(&libjvm, &[format!("-Djava.class.path={}", jar.display())])?;
    fx::register();
    let mut env = blazon_rt::vm::env()?;

    let first = fx::JavaString::new(&mut env, "first")?;
    let w: fx::Widget<fx::Object<'static>, fx::Object<'static>> =
        fx::Widget::with_count(&mut env, &first, 5)?;
    let hidden: fx::Widget<fx::Object<'static>, fx::Object<'static>> =
        fx::Widget::from_hidden(&mut env, "x")?;
    assert_eq!(
        hidden.count()?,
        1,
        "the private constructor sets count to 1"
    );
    assert_eq!(w.count()?, 5);
    w.set_count(9)?;
    assert_eq!(w.count()?, 9);
    assert_eq!(w.secret()?, "s");
    w.set_secret("t")?;
    assert_eq!(w.secret()?, "t");
    assert_eq!(w.hidden(2, "abc")?, 5);
    assert_eq!(fx::Widget::secret_static(&mut env, "x")?, "x!");
    assert_eq!(fx::Widget::MAX, 7);
    assert_eq!(fx::Widget::NAME, "widget");
    assert_eq!(fx::Widget::PRIVATE_COUNT, 3);

    assert_eq!(w.join(&["a", "b"])?, "a,b");
    assert_eq!(w.bytes(&[1, 2, 3])?, [1, 2, 3]);
    assert_eq!(w.names()?, ["a", "b"]);
    assert_eq!(w.boxed(41, true)?, 42);
    assert_eq!(w.scale(1.5, 0, 0, 0)?, 3.0);
    assert_eq!(w.workspace()?, "ws");
    assert_eq!(w.workspace_in("rel")?, "rel");
    assert!(w.find("x", None::<&Object>)?.is_none());
    let hint = fx::JavaString::new(&mut env, "h")?;
    assert!(w.find("x", Some(&hint))?.is_none());
    let key = fx::JavaString::new(&mut env, "key")?;
    w.put_key(Some(&key))?;
    assert_eq!(
        w.first("n")?.map(|k| k.to_string()),
        Some("key".to_string())
    );
    w.put_key(None::<&fx::JavaString>)?;
    assert!(w.first("n")?.is_none(), "nullable(0) takes None");
    w.put_key(Some(&key))?;
    let strings: List<fx::JavaString> = List::of_strs(&mut env, &["x"])?;
    assert_eq!(strings.len()?, 1);
    assert_eq!(
        w.label(Some(&key))?.map(|s| s.to_string()),
        Some("key".to_string())
    );
    assert_eq!(w.label(None::<&fx::JavaString>)?, None);
    let sink: fx::Map<fx::JavaString, fx::Integer> = fx::Map::new(&mut env)?;
    let numbers = w.mixed(&sink, &strings)?;
    assert_eq!(
        numbers.len()?,
        1,
        "a List<String> and a Map<String, Integer> go where `?` is expected"
    );
    let inner = w.inner()?;
    assert_eq!(inner.depth()?, 1);

    let base: fx::Base<fx::Object<'static>> = w.upcast();
    assert_eq!(base.held_field()?.to_string(), "key");

    let widget = w.to_global()?;
    let get = |expr: &str| {
        eval(expr, "w", |heap| {
            let mut env = blazon_rt::vm::env().expect("env");
            widget
                .get(&mut env)
                .expect("widget")
                .to_value(heap)
                .expect("value")
        })
    };
    assert_eq!(get("w.size"), "9");
    assert_eq!(get("w.scaled(\"a\", tag = \"b\")"), "\"a/b\"");
    assert_eq!(get("w.scaled(2)"), "None");
    assert_eq!(get("w.scaled(\"f\", tag = 3)"), "\"f/3\"");
    assert_eq!(get("w[\"k\"]"), "\"at:k\"");
    assert_eq!(get("\"k\" in w"), "True");
    assert_eq!(get("\"z\" in w"), "False");
    assert_eq!(get("type(w)"), "\"widget\"");
    assert_eq!(get("str(w)"), "\"Widget(9)\"");
    assert_eq!(get("w == w"), "True");
    assert_eq!(get("dir(w)"), "[\"scaled\", \"size\"]");
    assert!(get("w.nope").starts_with("ERR"), "{}", get("w.nope"));
    assert!(
        get("w.scaled(1, 2)").starts_with("ERR"),
        "{}",
        get("w.scaled(1, 2)")
    );
    unannotated(&mut env)?;
    failing_attribute(&mut env)?;
    throwables(&mut env)?;
    inherited(&mut env)?;
    primitives(&mut env)?;
    no_java_style_names();
    Ok(())
}

/// Classes with no `@StarlarkBuiltin`: facets from the spec's members.
fn unannotated(env: &mut jni::JNIEnv<'static>) -> Result<()> {
    let pkg = fx::Pkg::new(env)?;
    let (a_name, a_kind) = ("a", "cc_test");
    let (b_name, b_kind) = ("bb", "cc_library");
    let a = fx::Rule::new(env, a_name, a_kind)?;
    let b = fx::Rule::new(env, b_name, b_kind)?;
    pkg.add(&a)?;
    pkg.add(&b)?;
    let targets = pkg.targets()?;
    let mut names = Vec::new();
    for t in &targets {
        names.push(t?.name()?);
    }
    assert_eq!(names, ["a", "bb"]);
    let rule_a: fx::Target = a.upcast();
    assert_eq!(rule_a.name()?, "a");

    let pkg = pkg.to_global()?;
    let rule = a.to_global()?;
    let same = fx::Rule::new(env, a_name, a_kind)?.to_global()?;
    let run = |expr: &str| {
        let ast = AstModule::parse("t.star", expr.to_string(), &Dialect::Extended).expect("parse");
        Module::with_temp_heap(|module| {
            let mut env = blazon_rt::vm::env().expect("env");
            module.set(
                "p",
                pkg.get(&mut env)
                    .expect("pkg")
                    .to_value(module.heap())
                    .expect("value"),
            );
            module.set(
                "r",
                rule.get(&mut env)
                    .expect("rule")
                    .to_value(module.heap())
                    .expect("value"),
            );
            module.set(
                "same",
                same.get(&mut env)
                    .expect("rule")
                    .to_value(module.heap())
                    .expect("value"),
            );
            let mut eval = Evaluator::new(&module);
            Ok::<_, starlark::Error>(match eval.eval_module(ast, &Globals::standard()) {
                Ok(v) => v.to_string(),
                Err(e) => format!("ERR {e:#}"),
            })
        })
        .expect("heap")
    };
    assert_eq!(run("len(p.targets())"), "2");
    assert_eq!(run("[t.name for t in p.targets()]"), "[\"a\", \"bb\"]");
    assert_eq!(
        run("[t.rule_class for t in p.targets()]"),
        "[\"cc_test\", \"cc_library\"]"
    );
    assert_eq!(
        run("[type(t) for t in p.targets()]"),
        "[\"Rule\", \"Rule\"]"
    );
    assert_eq!(run("r in p.targets()"), "True");
    assert_eq!(run("same in p.targets()"), "True");
    assert_eq!(run("type(p)"), "\"Pkg\"");
    assert_eq!(run("type(p.targets())"), "\"Set\"");
    assert_eq!(run("r.is_test"), "True");
    assert_eq!(run("r.attribute_count()"), "1");
    assert_eq!(run("r.describe(\"x\", 3)"), "\"xxxa\"");
    assert_eq!(run("r == same"), "True");
    assert_eq!(run("{r: 1}[same]"), "1");
    assert_eq!(run("str(r)"), "\"Rule(a)\"");
    assert!(run("r.nope").starts_with("ERR"));
    Ok(())
}

/// `Throwable`, `Exception` and `RuntimeException` read a thrown object.
fn throwables(env: &mut jni::JNIEnv<'static>) -> Result<()> {
    use blazon_rt::Handle;
    use blazon_rt::java::lang::{Exception, RuntimeException, Throwable};
    let cause = env.new_object("java/io/IOException", "()V", &[])?;
    let msg = env.new_string("outer")?;
    let outer = env.new_object(
        "java/lang/IllegalStateException",
        "(Ljava/lang/String;Ljava/lang/Throwable;)V",
        &[(&msg).into(), (&cause).into()],
    )?;
    // SAFETY: an `IllegalStateException` is a `RuntimeException`.
    let rt: RuntimeException = unsafe { RuntimeException::from_raw(outer) };
    assert_eq!(rt.message()?.as_deref(), Some("outer"));
    assert_eq!(rt.class_name()?, "java.lang.IllegalStateException");
    assert!(
        rt.stack_trace()?
            .starts_with("java.lang.IllegalStateException: outer")
    );
    let cause: Throwable = rt.cause()?.expect("cause");
    assert_eq!(cause.class_name()?, "java.io.IOException");
    assert_eq!(cause.message()?, None);
    assert!(cause.cause()?.is_none());
    assert_eq!(rt.to_string(), "java.lang.IllegalStateException: outer");
    let e: Exception = rt.upcast();
    let t: Throwable = e.upcast();
    let boxed: Box<dyn std::error::Error> = Box::new(t);
    assert!(boxed.to_string().contains("outer"));
    Ok(())
}

/// An attribute whose read throws reports the exception, not a missing attribute.
fn failing_attribute(env: &mut jni::JNIEnv<'static>) -> Result<()> {
    let bag = fx::Bag::new(env)?.to_global()?;
    let run = |expr: &str| {
        eval(expr, "b", |heap| {
            let mut env = blazon_rt::vm::env().expect("env");
            bag.get(&mut env)
                .expect("bag")
                .to_value(heap)
                .expect("value")
        })
    };
    assert!(
        run("b.other").contains("no attribute `other`"),
        "{}",
        run("b.other")
    );
    for use_it in [
        "len(b.boom)",
        "b.boom + 1",
        "b.boom[0]",
        "b.boom()",
        "{b.boom: 1}",
    ] {
        let out = run(use_it);
        assert!(
            out.starts_with("ERR")
                && out.contains("boom from java")
                && out.contains("IllegalStateException")
                && !out.contains("no attribute"),
            "{use_it}: {out}"
        );
    }
    let shown = run("str(b.boom)");
    assert!(shown.contains("boom from java"), "{shown}");
    Ok(())
}

/// Members declared two levels up and on an interface, selected on the subclass
/// or on an ancestor, reach every bound descendant.
fn inherited(env: &mut jni::JNIEnv<'static>) -> Result<()> {
    let leaf = fx::Leaf::new(env)?;
    assert_eq!(
        leaf.top_value()?,
        "top",
        "declared on Top, two levels up, from an ancestor's spec"
    );
    assert_eq!(leaf.leaf_value()?, "leaf");
    assert_eq!(
        leaf.mark_of_leaf()?,
        "mark",
        "selected on the subclass and resolved through the chain"
    );
    assert_eq!(
        leaf.hidden()?,
        "h",
        "a private field of Top, reached through Top's class"
    );
    assert_eq!(
        leaf.protected_value()?,
        "protected",
        "a protected method of Top"
    );
    let mid: fx::Mid = leaf.upcast();
    assert_eq!(
        mid.top_value()?,
        "top",
        "Mid lists no members and still has Top's"
    );
    let global = leaf.to_global()?;
    let run = |expr: &str| {
        let ast = AstModule::parse("t.star", expr.to_string(), &Dialect::Extended).expect("parse");
        Module::with_temp_heap(|module| {
            let mut env = blazon_rt::vm::env().expect("env");
            module.set(
                "x",
                global
                    .get(&mut env)
                    .expect("leaf")
                    .to_value(module.heap())
                    .expect("value"),
            );
            let mut eval = Evaluator::new(&module);
            Ok::<_, starlark::Error>(match eval.eval_module(ast, &Globals::standard()) {
                Ok(v) => v.to_string(),
                Err(e) => format!("ERR {e:#}"),
            })
        })
        .expect("heap")
    };
    assert_eq!(run("x.top_value"), "\"top\"");
    assert_eq!(run("x.leaf_value"), "\"leaf\"");
    assert_eq!(run("x.mark_of_leaf()"), "\"mark\"");
    assert_eq!(run("type(x)"), "\"Leaf\"");
    Ok(())
}

/// Every Java primitive, as a parameter and as a result.
fn primitives(env: &mut jni::JNIEnv<'static>) -> Result<()> {
    let prims = fx::Prims::new(env)?;
    assert!(!prims.flip(true)?);
    assert_eq!(prims.inc(127)?, -128);
    assert_eq!(prims.next(b'a' as u16)?, b'b' as u16);
    assert_eq!(prims.half(3.0)?, 1.5);
    let global = prims.to_global()?;
    let run = |expr: &str| {
        let ast = AstModule::parse("t.star", expr.to_string(), &Dialect::Extended).expect("parse");
        Module::with_temp_heap(|module| {
            let mut env = blazon_rt::vm::env().expect("env");
            module.set(
                "x",
                global
                    .get(&mut env)
                    .expect("prims")
                    .to_value(module.heap())
                    .expect("value"),
            );
            let mut eval = Evaluator::new(&module);
            Ok::<_, starlark::Error>(match eval.eval_module(ast, &Globals::standard()) {
                Ok(v) => v.to_string(),
                Err(e) => format!("ERR {e:#}"),
            })
        })
        .expect("heap")
    };
    assert_eq!(run("x.flip(True)"), "False");
    assert_eq!(run("x.inc(41)"), "42");
    assert_eq!(run("x.inc(127)"), "-128");
    assert!(
        run("x.inc(200)").contains("out of range for a byte"),
        "{}",
        run("x.inc(200)")
    );
    assert!(run("x.inc(-129)").starts_with("ERR"));
    assert_eq!(run("x.sinc(-32768)"), "-32767");
    assert!(run("x.sinc(40000)").contains("out of range for a short"));
    assert_eq!(run("x.next(\"a\")"), "\"b\"");
    assert!(
        run("x.next(\"ab\")").contains("one-character"),
        "{}",
        run("x.next(\"ab\")")
    );
    assert!(run("x.next(\"\")").starts_with("ERR"));
    assert_eq!(run("x.twice(21)"), "42");
    assert_eq!(run("x.big(1)"), "1048576");
    assert_eq!(run("x.half(3)"), "1.5");
    assert_eq!(run("x.half(3.0)"), "1.5");
    assert_eq!(run("x.scale(2.5)"), "2.5");
    assert_eq!(run("x.set_timeout_scaling(2)"), "None");
    assert_eq!(run("x.scale(2.5)"), "5.0");
    assert_eq!(
        run("x.all(True, 1, 2, \"c\", 4, 5, 6.5, 7.25)"),
        "\"true12c456.57.25\""
    );
    assert!(run("x.twice(\"a\")").starts_with("ERR"));
    assert!(run("x.twice(3000000000)").starts_with("ERR"));
    Ok(())
}

/// Scripts never see Java-style names: every member of a generated type, in `dir()`,
/// is snake_case.
fn no_java_style_names() {
    use starlark::docs::DocType;
    fn check(type_name: &str, doc: DocType) {
        assert!(!doc.members.is_empty(), "{type_name} has members");
        for name in doc.members.keys() {
            assert!(
                !name.chars().any(|c| c.is_uppercase()),
                "{type_name}.{name} is camelCase"
            );
        }
    }
    check("Widget", DocType::from_starlark_value::<fx::WidgetValue>());
    check("Rule", DocType::from_starlark_value::<fx::RuleValue>());
    check("Target", DocType::from_starlark_value::<fx::TargetValue>());
    check("Pkg", DocType::from_starlark_value::<fx::PkgValue>());
    check("Leaf", DocType::from_starlark_value::<fx::LeafValue>());
    check("Prims", DocType::from_starlark_value::<fx::PrimsValue>());
}
