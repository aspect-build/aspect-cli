//! Implementing Java types from Rust, at generation time: which abstract
//! methods become the trait, what the generated class file looks like, and the
//! errors for what blazon cannot implement. Running the result in a JVM is
//! `blazon-selftest`'s job.

use blazon::{Version, generate};
use cafebabe::{MethodAccessFlags, parse_class};
use std::fmt::Write;
use std::path::PathBuf;

fn dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests")
}

fn run(spec: &str, name: &str) -> anyhow::Result<blazon::Report> {
    let out = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join(name);
    std::fs::create_dir_all(&out)?;
    let spec_path = out.join("spec.toml");
    std::fs::write(&spec_path, spec)?;
    generate(
        &spec_path,
        &[Version {
            label: "fixture".into(),
            jar: dir().join("fixture/fixture.jar"),
        }],
        &out.join("bindings"),
    )
}

fn failure(spec: &str, name: &str) -> String {
    match run(spec, name) {
        Ok(_) => panic!("{name}: generation succeeded"),
        Err(e) => format!("{e:#}"),
    }
}

const TARGET: &str = r#"
[[class]]
java = "fx.Target"
members = []
"#;

#[test]
fn an_interface_becomes_a_trait_with_every_abstract_method() -> anyhow::Result<()> {
    let spec =
        format!("versions = [\"fixture\"]\n{TARGET}\n[[implement]]\njava = \"fx.Callback\"\n");
    let report = run(&spec, "implement-all")?;
    let [callback] = report.implementations.as_slice() else {
        panic!("one implementation");
    };
    let names: Vec<&str> = callback
        .methods
        .iter()
        .map(|m| m.rust_name.as_str())
        .collect();
    assert_eq!(
        names,
        [
            "add",
            "mix",
            "greet",
            "maybe",
            "parent",
            "maybe_parent",
            "names",
            "identity",
            "boxed",
            "notify",
            "bytes",
            "strings",
            "maybe_bytes",
            "opt",
            "fail",
            "explode",
            "slow"
        ],
        "the default and static methods are not implemented"
    );
    assert_eq!(callback.trait_name, "CallbackImpl");
    assert_eq!(callback.handle, "Callback");
    assert!(
        callback.class.starts_with("blazon/impl/Callback$"),
        "{}",
        callback.class
    );

    let cf = parse_class(&callback.class_bytes).expect("a class file");
    let interfaces: Vec<String> = cf.interfaces.iter().map(|i| i.to_string()).collect();
    assert_eq!(interfaces, ["fx/Callback"]);
    let native: Vec<(&str, String)> = cf
        .methods
        .iter()
        .filter(|m| m.access_flags.contains(MethodAccessFlags::NATIVE))
        .map(|m| (m.name.as_ref(), m.descriptor.to_string()))
        .collect();
    assert_eq!(native.len(), names.len());
    assert!(native.contains(&(
        "identity",
        "(Ljava/lang/Object;)Ljava/lang/Object;".to_string()
    )));
    assert!(native.contains(&("mix", "(JDZBCSF)J".to_string())));
    assert!(
        !cf.methods
            .iter()
            .any(|m| m.name == "describe" || m.name == "fixed")
    );
    Ok(())
}

#[test]
fn members_select_and_rename_abstract_methods() -> anyhow::Result<()> {
    let spec = format!(
        r#"versions = ["fixture"]
{TARGET}
[[implement]]
java = "fx.Callback"
rust = "Calls"
members = ["add", "greet as hello", "maybe?"]
"#
    );
    let report = run(&spec, "implement-select")?;
    let i = &report.implementations[0];
    assert_eq!(i.trait_name, "Calls");
    let names: Vec<&str> = i.methods.iter().map(|m| m.rust_name.as_str()).collect();
    assert_eq!(names, ["add", "hello", "maybe"]);
    let code = std::fs::read_to_string(
        PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join("implement-select/bindings/vfixture.rs"),
    )?;
    assert!(code.contains("pub trait Calls: Send + Sync"), "{code}");
    assert!(
        !code.contains("fn explode"),
        "unselected methods are not in the trait"
    );
    Ok(())
}

#[test]
fn an_abstract_class_is_extended_and_protected_methods_are_implemented() -> anyhow::Result<()> {
    let spec = r#"
versions = ["fixture"]

[[implement]]
java = "fx.Task"
"#;
    let report = run(spec, "implement-class")?;
    let i = &report.implementations[0];
    assert_eq!(i.super_class, "fx/Task");
    let names: Vec<&str> = i.methods.iter().map(|m| m.rust_name.as_str()).collect();
    assert_eq!(
        names,
        ["run", "priority"],
        "the concrete `name` is left alone"
    );
    let cf = parse_class(&i.class_bytes).expect("a class file");
    assert_eq!(
        cf.super_class.map(|c| c.to_string()).as_deref(),
        Some("fx/Task")
    );
    assert!(cf.interfaces.is_empty());
    Ok(())
}

#[test]
fn a_class_file_snapshot_shows_accidental_changes() -> anyhow::Result<()> {
    let spec = format!(
        r#"versions = ["fixture"]
{TARGET}
[[implement]]
java = "fx.Callback"

[[implement]]
java = "fx.Task"
"#
    );
    let report = run(&spec, "implement-snapshot")?;
    let mut got = String::new();
    for i in &report.implementations {
        let _ = writeln!(
            got,
            "{} {} {} bytes {}",
            i.java,
            i.class,
            i.class_bytes.len(),
            blazon::classwriter::digest(&i.class_bytes)
        );
    }
    let free = &report.implementations[0].free_bytes;
    let _ = writeln!(
        got,
        "{} {} bytes {}",
        blazon::classwriter::FREE,
        free.len(),
        blazon::classwriter::digest(free)
    );
    let want_path = dir().join("snapshots/implement.txt");
    if std::env::var_os("BLAZON_BLESS").is_some() {
        std::fs::write(&want_path, &got)?;
    }
    let want = std::fs::read_to_string(&want_path).unwrap_or_default();
    assert!(
        got == want,
        "the generated class files differ from {}; rerun with BLAZON_BLESS=1 to accept\n{got}",
        want_path.display()
    );
    Ok(())
}

#[test]
fn what_cannot_be_implemented_is_an_error_that_says_why() {
    let text = failure(
        r#"
versions = ["fixture"]

[[implement]]
java = "fx.NoCtor"
"#,
        "implement-no-ctor",
    );
    assert!(
        text.contains("fx.NoCtor") && text.contains("no no-argument constructor"),
        "{text}"
    );

    let text = failure(
        r#"
versions = ["fixture"]

[[implement]]
java = "fx.PackageAbstract"
"#,
        "implement-package",
    );
    assert!(
        text.contains("package-private") && text.contains("hidden"),
        "{text}"
    );

    let text = failure(
        r#"
versions = ["fixture"]

[[implement]]
java = "fx.Hidden"
"#,
        "implement-hidden",
    );
    assert!(text.contains("not public"), "{text}");

    let text = failure(
        r#"
versions = ["fixture"]

[[implement]]
java = "java.lang.Runnable"
"#,
        "implement-jdk",
    );
    assert!(text.contains("JDK"), "{text}");

    let text = failure(
        r#"
versions = ["fixture"]

[[implement]]
java = "fx.Callback"
members = ["nothing"]
"#,
        "implement-member",
    );
    assert!(
        text.contains("`nothing` is not an abstract method") && text.contains("add("),
        "{text}"
    );

    let text = failure(
        r#"
versions = ["fixture"]

[[implement]]
java = "fx.Callback"
extends = "fx.NoCtor"
"#,
        "implement-extends",
    );
    assert!(text.contains("fx.NoCtor"), "{text}");
}

#[test]
fn a_type_the_trait_mentions_must_be_bound() {
    let text = failure(
        r#"
versions = ["fixture"]

[[implement]]
java = "fx.Callback"
"#,
        "implement-unbound",
    );
    assert!(text.contains("fx.Target is not bound"), "{text}");
}

#[test]
fn the_class_name_follows_the_methods() -> anyhow::Result<()> {
    let both =
        format!("versions = [\"fixture\"]\n{TARGET}\n[[implement]]\njava = \"fx.Callback\"\n");
    let fewer = format!(
        "versions = [\"fixture\"]\n{TARGET}\n[[implement]]\njava = \"fx.Callback\"\nmembers = [\"add\"]\n"
    );
    let a = run(&both, "implement-name-a")?.implementations[0]
        .class
        .clone();
    let b = run(&fewer, "implement-name-b")?.implementations[0]
        .class
        .clone();
    assert_ne!(a, b, "another set of methods is another class");
    Ok(())
}

/// buildx's `[[implement]]` entries on Bazel 9.0.1: the abstract methods Bazel
/// asks for, and a snapshot of the class files. Skipped without the release.
#[test]
fn buildx_implementations_on_bazel_9_0_1() -> anyhow::Result<()> {
    let Some(home) = std::env::var_os("HOME") else {
        return Ok(());
    };
    let jar = PathBuf::from(home).join("Library/Caches/buildx/bazel/9.0.1/A-server.jar");
    if !jar.exists() {
        return Ok(());
    }
    let out = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join("implement-buildx");
    let report = generate(
        &dir().join("../../buildx/bindings.toml"),
        &[Version {
            label: "9.0.1".into(),
            jar,
        }],
        &out,
    )?;
    let mut got = String::new();
    for i in &report.implementations {
        let methods: Vec<&str> = i.methods.iter().map(|m| m.rust_name.as_str()).collect();
        let _ = writeln!(
            got,
            "{} {} {} bytes {} [{}]",
            i.java,
            i.class,
            i.class_bytes.len(),
            blazon::classwriter::digest(&i.class_bytes),
            methods.join(", ")
        );
    }
    let find = |name: &str| {
        report
            .implementations
            .iter()
            .find(|i| i.java.ends_with(name))
            .unwrap_or_else(|| panic!("{name} is implemented"))
    };
    let names = |i: &blazon::ir::Implementation| -> Vec<String> {
        i.methods.iter().map(|m| m.rust_name.clone()).collect()
    };
    assert_eq!(
        names(find("CppIncludeExtractionContext")),
        ["artifact_resolver"]
    );
    assert_eq!(
        names(find("CppIncludeScanningContext")),
        ["find_additional_inputs"]
    );
    assert_eq!(names(find("SpawnStrategy")), ["exec", "can_exec"]);
    let want_path = dir().join("snapshots/implement_bazel.txt");
    if std::env::var_os("BLAZON_BLESS").is_some() {
        std::fs::write(&want_path, &got)?;
    }
    let want = std::fs::read_to_string(&want_path).unwrap_or_default();
    assert!(
        got == want,
        "the generated class files differ from {}; rerun with BLAZON_BLESS=1 to accept\n{got}",
        want_path.display()
    );
    Ok(())
}
