//! Bazel 9.0.1's own classes through generated bindings and Starlark types:
//! `Label`, `Dict`, and the private members of `BazelRepositoryModule`.
//! Skipped when the release or a `libjvm` is not available.

#![cfg(not(no_bazel))]

use blazon_rt::{Jvm, Result};
use blazon_selftest::bazel::v9_0_1 as bz;
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

fn jar() -> Option<std::path::PathBuf> {
    let home = std::env::var_os("HOME")?;
    let jar = std::path::PathBuf::from(home).join("Library/Caches/buildx/bazel/9.0.1/A-server.jar");
    jar.exists().then_some(jar)
}

#[test]
fn bazel() -> Result<()> {
    let (Some(libjvm), Some(jar)) = (blazon_selftest::libjvm(), jar()) else {
        eprintln!("skipped: no libjvm or Bazel 9.0.1");
        return Ok(());
    };
    let _jvm = Jvm::start(&libjvm, &[format!("-Djava.class.path={}", jar.display())])?;
    bz::register();
    let mut env = blazon_rt::vm::env()?;
    label(&mut env)?;
    dict(&mut env)?;
    unannotated(&mut env)?;
    inherited();
    repository_module(&mut env)?;
    Ok(())
}

fn label(env: &mut jni::JNIEnv<'static>) -> Result<()> {
    let label = bz::Label::parse_canonical(env, "@@repo//pkg/sub:target")?.to_global()?;
    for (expr, want) in [
        ("l.name", "\"target\""),
        ("l.package", "\"pkg/sub\""),
        ("l.repo_name", "\"repo\""),
        ("l.workspace_name", "\"repo\""),
        ("str(l)", "\"Label(\\\"@@repo//pkg/sub:target\\\")\""),
        ("type(l)", "\"Label\""),
        ("l == l", "True"),
        ("bool(l)", "True"),
        ("l.same_package_label(\"x\").name", "\"x\""),
        (
            "dir(l)",
            "[\"name\", \"package\", \"relative\", \"repo_name\", \"same_package_label\", \"workspace_name\", \"workspace_root\"]",
        ),
    ] {
        let got = eval(expr, "l", |heap| {
            let mut env = blazon_rt::vm::env().expect("env");
            label
                .get(&mut env)
                .expect("label")
                .to_value(heap)
                .expect("value")
        });
        assert_eq!(got, want, "{expr}");
    }
    // `relative` needs a Starlark context a bare thread does not have; the Java exception reaches the script.
    let got = eval("l.relative(\"other\")", "l", |heap| {
        let mut env = blazon_rt::vm::env().expect("env");
        label
            .get(&mut env)
            .expect("label")
            .to_value(heap)
            .expect("value")
    });
    assert!(
        got.contains("NullPointerException") && got.contains("checkNotNull"),
        "{got}"
    );
    Ok(())
}

fn dict(env: &mut jni::JNIEnv<'static>) -> Result<()> {
    let map: bz::Map<bz::Object<'static>, bz::Object<'static>> = bz::Map::new(env)?;
    let (k, v) = (
        bz::JavaString::new(env, "k")?,
        bz::JavaString::new(env, "v")?,
    );
    map.put(&k, &v)?;
    let dict = bz::Dict::copy_of(env, &map)?.to_global()?;
    for (expr, want) in [
        ("d[\"k\"]", "\"v\""),
        ("len(d)", "1"),
        ("\"k\" in d", "True"),
        ("\"z\" in d", "False"),
        ("d.get(\"zz\", 1)", "1"),
        ("d.get(\"k\")", "\"v\""),
        ("list(d)", "[\"k\"]"),
        ("d.keys()", "[\"k\"]"),
        ("d.keys()[0]", "\"k\""),
        ("d.items()", "[(\"k\", \"v\")]"),
        ("type(d)", "\"dict\""),
    ] {
        let got = eval(expr, "d", |heap| {
            let mut env = blazon_rt::vm::env().expect("env");
            dict.get(&mut env)
                .expect("dict")
                .to_value(heap)
                .expect("value")
        });
        assert_eq!(got, want, "{expr}");
    }
    Ok(())
}

fn repository_module(env: &mut jni::JNIEnv<'static>) -> Result<()> {
    let module = bz::BazelRepositoryModule::new(env)?;
    let cache = module.repository_cache()?;
    let new_cache = bz::RepositoryCache::new(env)?;
    module.set_repository_cache(&new_cache)?;
    assert_eq!(module.repository_cache()?, new_cache);
    assert_ne!(module.repository_cache()?, cache);

    let supplier = module.repo_env_supplier()?;
    let (k, v) = (
        bz::JavaString::new(env, "K")?,
        bz::JavaString::new(env, "V")?,
    );
    let m: bz::ImmutableMap<bz::JavaString, bz::JavaString> = bz::ImmutableMap::of(env, &k, &v)?;
    supplier.set(&m)?;
    let got = supplier.get()?;
    assert_eq!(
        got.get(Some(&k))?.map(|v| v.to_string()),
        Some("V".to_string())
    );
    assert!(got.get(Some(&v))?.is_none());

    let empty: bz::ImmutableMap<bz::JavaString, bz::JavaString> = bz::ImmutableMap::empty(env)?;
    module.nonstrict_repo_env_supplier()?.set(&empty)?;
    let nonstrict: bz::Map<bz::JavaString, bz::JavaString> =
        module.nonstrict_repo_env_supplier()?.get()?.upcast();
    assert_eq!(nonstrict.len()?, 0);

    let clock = bz::HttpDownloader::clock(env)?;
    module.set_clock(Some(&clock))?;
    assert_eq!(module.clock()?, Some(clock));
    assert!(module.registries()?.is_none());
    assert!(module.module_mirrors()?.is_none());
    assert!(module.repository_fetch_function()?.is_none());
    assert!(module.module_file_function()?.is_none());
    assert!(module.repo_spec_function()?.is_none());
    assert!(module.yanked_versions_function()?.is_none());
    let _ = module.vendor_command()?;

    let d = blazon_rt::java::time::Duration::new(env, std::time::Duration::from_secs(3))?;
    let http = bz::HttpDownloader::new(env, 2, &d, 3, 1.5)?;
    let _: bz::Downloader = http.upcast();
    Ok(())
}

/// Bazel classes without `@StarlarkBuiltin`, with facets from the spec's members and ancestors.
fn unannotated(env: &mut jni::JNIEnv<'static>) -> Result<()> {
    let path = bz::PathFragment::create(env, "a/b")?.to_global()?;
    let other = bz::PathFragment::create(env, "a/b")?.to_global()?;
    let one = bz::JavaString::new(env, "x")?;
    let made: bz::ImmutableSet<bz::JavaString> = bz::ImmutableSet::of(env, &one)?;
    let set = made.to_global()?;
    let run = |expr: &str| {
        let ast = AstModule::parse("t.star", expr.to_string(), &Dialect::Extended).expect("parse");
        Module::with_temp_heap(|module| {
            let mut env = blazon_rt::vm::env().expect("env");
            module.set(
                "p",
                path.get(&mut env)
                    .expect("p")
                    .to_value(module.heap())
                    .expect("value"),
            );
            module.set(
                "q",
                other
                    .get(&mut env)
                    .expect("q")
                    .to_value(module.heap())
                    .expect("value"),
            );
            module.set(
                "s",
                set.get(&mut env)
                    .expect("s")
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
    assert_eq!(run("p.path_string"), "\"a/b\"");
    assert_eq!(run("p.is_absolute"), "False");
    assert_eq!(run("p.segmentCount()"), "2");
    assert_eq!(run("p.getParentDirectory().path_string"), "\"a\"");
    assert_eq!(run("p.getRelative(\"c\").path_string"), "\"a/b/c\"");
    assert_eq!(run("type(p)"), "\"PathFragment\"");
    assert_eq!(run("p == q"), "True");
    assert_eq!(run("{p: 1}[q]"), "1");
    assert_eq!(run("type(s)"), "\"ImmutableSet\"");
    assert_eq!(run("len(s)"), "1");
    assert_eq!(run("list(s)"), "[\"x\"]");
    assert_eq!(run("\"x\" in s"), "True");
    assert_eq!(run("\"y\" in s"), "False");
    assert_eq!(run("bool(s)"), "True");
    Ok(())
}

/// Members declared on ancestors (`RuleOrMacroInstance`, `Target`, `FileTarget`, `Packageoid`)
/// are on the subclasses' Starlark types.
fn inherited() {
    use starlark::docs::DocType;
    let members = |doc: DocType| doc.members.keys().cloned().collect::<Vec<_>>();
    let rule = members(DocType::from_starlark_value::<bz::RuleValue>());
    for want in ["label", "name", "getAttr", "rule_class", "visibility"] {
        assert!(rule.iter().any(|m| m == want), "Rule has {want}: {rule:?}");
    }
    let input = members(DocType::from_starlark_value::<bz::InputFileValue>());
    assert!(
        input.iter().any(|m| m == "label") && input.iter().any(|m| m == "visibility"),
        "{input:?}"
    );
    let package = members(DocType::from_starlark_value::<bz::PackageValue>());
    assert!(package.iter().any(|m| m == "targets"), "{package:?}");
}
