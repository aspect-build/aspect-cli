//! Generation over a small class-file fixture (`tests/fixture/src`, built into
//! `fixture.jar`): generics, wildcards, arrays, varargs, nullability, inner
//! classes, overloads, statics, constants, private members, and Starlark
//! annotations. The emitted code is compared with a snapshot
//! (`BLAZON_BLESS=1` rewrites it) and compiled against `blazon-rt` and
//! `blazon-starlark`.

use blazon::{Version, generate};
use std::path::{Path, PathBuf};
use std::process::Command;

fn dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests")
}

const SPEC: &str = r#"
versions = ["fixture"]
starlark = true

[[class]]
java = "fx.Widget"
members = [
    "<init>() as new",
    "<init>(Ljava/lang/Object;I) as with_count",
    "<init>(Ljava/lang/String;) as from_hidden",
    "of",
    "put(Ljava/lang/Object;) as put_key nullable(0)",
    "put(Ljava/lang/Object;Ljava/lang/Comparable;) as put_entry",
    "find?",
    "join",
    "things",
    "bytes",
    "names",
    "label",
    "mixed",
    "boxed",
    "inner",
    "isReady",
    "hasMore",
    "getWorkspace()",
    "getWorkspace(Ljava/lang/String;) as workspace_in",
    "setLimit",
    "hidden",
    "secretStatic",
    "secret",
    "count",
    "index",
    "PRIVATE_COUNT",
    "MAX",
    "NAME",
    "BIG",
    "FLAG",
    "RATIO",
    "SHARED",
]

[[class]]
java = "fx.Widget$Inner"
rust = "WidgetInner"
members = ["depth"]

[[class]]
java = "fx.Base"
members = ["held()", ".held as held_field"]

[[class]]
java = "fx.Prims"
members = [
    "<init>()", "flip", "inc", "sinc", "next", "twice", "big", "half", "scale",
    "setTimeoutScaling", "all",
]

[[class]]
java = "fx.Target"
members = []

[[implement]]
java = "fx.Callback"

[[implement]]
java = "fx.Task"

[[class]]
java = "net.starlark.java.eval.Starlark"
members = []

[[class]]
java = "net.starlark.java.eval.StarlarkValue"
members = []

[options]
fragments = true
include_effect_tags = ["LOADING_AND_ANALYSIS"]
exclude_effect_tags = ["EXECUTION"]
exclude_classes = ["fx.opts.RemoteStuff", "fx.opts.Dup"]
"#;

fn generate_into(spec: &str, out: &Path) -> anyhow::Result<()> {
    let spec_path = out.join("spec.toml");
    std::fs::create_dir_all(out)?;
    std::fs::write(&spec_path, spec)?;
    generate(
        &spec_path,
        &[Version {
            label: "fixture".into(),
            jar: dir().join("fixture/fixture.jar"),
        }],
        &out.join("bindings"),
    )?;
    Ok(())
}

fn tmp(name: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join(name)
}

#[test]
fn snapshot() -> anyhow::Result<()> {
    let out = tmp("snapshot");
    generate_into(SPEC, &out)?;
    let got = std::fs::read_to_string(out.join("bindings/vfixture.rs"))?;
    let want_path = dir().join("snapshots/fixture.rs.txt");
    if std::env::var_os("BLAZON_BLESS").is_some() {
        std::fs::write(&want_path, &got)?;
    }
    let want = std::fs::read_to_string(&want_path).unwrap_or_default();
    assert!(
        got == want,
        "generated code differs from {}; rerun with BLAZON_BLESS=1 to accept",
        want_path.display()
    );
    Ok(())
}

#[test]
fn unsupported_members_are_named() {
    let out = tmp("unsupported");
    let spec = r#"
versions = ["fixture"]

[[class]]
java = "fx.Widget"
members = ["numbers"]
"#;
    let err = generate_into(spec, &out).unwrap_err();
    let text = format!("{err:#}");
    assert!(text.contains("Bazel fixture"), "{text}");
    assert!(text.contains("fx.Widget.numbers"), "{text}");
    assert!(text.contains("array of int"), "{text}");
}

#[test]
fn ambiguous_overloads_are_listed() {
    let out = tmp("ambiguous");
    let spec = r#"
versions = ["fixture"]

[[class]]
java = "fx.Widget"
members = ["put"]
"#;
    let err = generate_into(spec, &out).unwrap_err();
    let text = format!("{err:#}");
    assert!(text.contains("has 2 overloads"), "{text}");
    assert!(
        text.contains("put(Ljava/lang/Object;Ljava/lang/Comparable;)"),
        "{text}"
    );
}

#[test]
fn a_class_missing_from_the_jar_names_the_spec_entry() {
    let out = tmp("missing");
    let spec = r#"
versions = ["fixture"]

[[class]]
java = "fx.NoSuchClass"
members = []
"#;
    let err = generate_into(spec, &out).unwrap_err();
    let text = format!("{err:#}");
    assert!(text.contains("spec entry `fx.NoSuchClass`"), "{text}");
}

#[test]
fn a_member_found_nowhere_in_the_chain_is_an_error() {
    let out = tmp("nowhere");
    let spec = r#"
versions = ["fixture"]

[[class]]
java = "fx.Leaf"
members = ["getTopValue", "noSuchThing"]
"#;
    let err = generate_into(spec, &out).unwrap_err();
    let text = format!("{err:#}");
    assert!(text.contains("fx.Leaf"), "{text}");
    assert!(text.contains("noSuchThing"), "{text}");
    assert!(text.contains("any of its supertypes"), "{text}");
}

#[test]
fn members_resolve_through_superclasses_and_interfaces() -> anyhow::Result<()> {
    let out = tmp("chain");
    let spec = r#"
versions = ["fixture"]

[[class]]
java = "fx.Top"
members = ["getTopValue"]

[[class]]
java = "fx.Mid"
members = []

[[class]]
java = "fx.Leaf"
members = ["getMark", ".hidden"]
"#;
    generate_into(spec, &out)?;
    let code = std::fs::read_to_string(out.join("bindings/vfixture.rs"))?;
    let squash: String = code.split_whitespace().collect();
    // Mid and Leaf list nothing from Top, and have it; Leaf's private field is read through Top.
    assert_eq!(
        squash.matches("pubfntop_value(").count(),
        3,
        "Top, Mid and Leaf"
    );
    assert_eq!(
        squash.matches("\"getTopValue\"").count(),
        3,
        "one call per class"
    );
    assert!(
        squash.contains("staticDECL"),
        "the private field is reflected on its declaring class"
    );
    Ok(())
}

#[test]
fn a_class_listed_twice_is_an_error() {
    let out = tmp("twice");
    let spec = r#"
versions = ["fixture"]

[[class]]
java = "fx.Base"
members = []

[[class]]
java = "fx.Base"
members = []
"#;
    let err = generate_into(spec, &out).unwrap_err();
    assert!(format!("{err:#}").contains("listed twice"), "{err:#}");
}

#[test]
fn standard_types_are_not_generated() {
    let out = tmp("jdk");
    let spec = r#"
versions = ["fixture"]

[[class]]
java = "java.util.List"
members = []
"#;
    let err = generate_into(spec, &out).unwrap_err();
    assert!(format!("{err:#}").contains("blazon_rt::java"), "{err:#}");
}

#[test]
fn names_that_every_handle_has_need_an_alias() {
    let out = tmp("reserved");
    let spec = r#"
versions = ["fixture"]

[[class]]
java = "fx.Widget"
members = ["hidden as upcast"]
"#;
    let err = generate_into(spec, &out).unwrap_err();
    assert!(
        format!("{err:#}").contains("every handle already has"),
        "{err:#}"
    );
}

/// Builds the emitted code against the runtime crates.
#[test]
fn emitted_code_compiles() -> anyhow::Result<()> {
    if std::env::var_os("BLAZON_SKIP_COMPILE").is_some() {
        return Ok(());
    }
    let out = tmp("compile");
    generate_into(SPEC, &out)?;
    let crates = dir().join("../..");
    let project = out.join("project");
    std::fs::create_dir_all(project.join("src"))?;
    std::fs::write(
        project.join("Cargo.toml"),
        format!(
            r#"[package]
name = "emitted"
version = "0.0.0"
edition = "2024"

[workspace]

[dependencies]
allocative = "0.3.4"
blazon-rt = {{ path = {:?} }}
blazon-starlark = {{ path = {:?} }}
jni = "0.21"
starlark = "0.14.2"
"#,
            crates.join("blazon-rt"),
            crates.join("blazon-starlark")
        ),
    )?;
    let bindings = out.join("bindings/vfixture.rs");
    std::fs::write(
        project.join("src/lib.rs"),
        format!("#[path = {bindings:?}]\npub mod bindings;\n"),
    )?;
    let status = Command::new(env!("CARGO"))
        .args(["check", "--offline", "--quiet"])
        .arg("--manifest-path")
        .arg(project.join("Cargo.toml"))
        .env("CARGO_TARGET_DIR", tmp("compile-target"))
        .env("RUSTFLAGS", "-D warnings")
        .status()?;
    assert!(status.success(), "the emitted code does not compile");
    Ok(())
}
