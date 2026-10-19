//! Per-version members: a ` when` condition on a member, and `versions` on a
//! class or an implementation, bind it only in the matching version modules.
//! The fixture jar stands in for two releases that differ, labelled `a` and `b`;
//! the live test is Bazel 9.0.1 and 9.2.0.

use blazon::{Version, generate};
use std::path::PathBuf;

fn dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests")
}

fn versions(labels: &[&str]) -> Vec<Version> {
    labels
        .iter()
        .map(|l| Version {
            label: (*l).into(),
            jar: dir().join("fixture/fixture.jar"),
        })
        .collect()
}

fn run(spec: &str, name: &str) -> anyhow::Result<(blazon::Report, PathBuf)> {
    let out = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join(name);
    std::fs::create_dir_all(&out)?;
    let spec_path = out.join("spec.toml");
    std::fs::write(&spec_path, spec)?;
    let report = generate(&spec_path, &versions(&["a", "b"]), &out.join("bindings"))?;
    Ok((report, out.join("bindings")))
}

const SPEC: &str = r#"
versions = ["a", "b"]

[[class]]
java = "fx.Widget"
members = [
    "<init>() as new when a",
    "<init>(Ljava/lang/Object;I) as new when b",
    "isReady",
    "numbers when c",
]

[[class]]
java = "fx.Prims"
versions = ["b"]
members = ["<init>()"]

[[class]]
java = "fx.Target"
members = []

[[implement]]
java = "fx.Callback"
versions = ["a"]
members = ["add"]

[[implement]]
java = "fx.Task"
members = ["run", "priority when b"]
"#;

#[test]
fn members_and_classes_are_bound_only_where_their_condition_says() -> anyhow::Result<()> {
    let (report, out) = run(SPEC, "versions")?;
    let a = std::fs::read_to_string(out.join("va.rs"))?;
    let b = std::fs::read_to_string(out.join("vb.rs"))?;

    let widget_new = |code: &str| -> String {
        let start = code.find("pub fn new<'l>(").expect("a constructor");
        let end = code[start..].find(") ->").expect("end of parameters");
        code[start..start + end].split_whitespace().collect()
    };
    assert!(
        !widget_new(&a).contains("p0:"),
        "a binds the no-argument constructor: {a}"
    );
    assert!(widget_new(&b).contains("p1:i32"), "b binds (Object, int)");

    assert!(
        !a.contains("pub struct Prims") && !a.contains("CLASS_PRIMS"),
        "Prims is only in b"
    );
    assert!(b.contains("CLASS_PRIMS"));
    assert!(a.contains("trait CallbackImpl") && !b.contains("trait CallbackImpl"));
    assert!(a.contains("fn add<'a>"), "{a}");
    assert!(
        !a.contains("fn priority") && b.contains("fn priority"),
        "a method of an implementation is conditional too"
    );

    let shape = report.shape.expect("two versions have a report");
    assert!(shape.contains("only in a  fx.Widget.<init>()"), "{shape}");
    assert!(
        shape.contains("only in b  fx.Widget.<init>(Ljava/lang/Object;I)"),
        "{shape}"
    );
    assert!(!a.contains("numbers") && !b.contains("numbers"));
    assert!(shape.contains("same     fx.Widget.isReady"), "{shape}");
    Ok(())
}

#[test]
fn a_member_without_a_condition_that_is_missing_is_still_an_error() {
    let out = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join("versions-missing");
    std::fs::create_dir_all(&out).expect("dir");
    let spec = r#"
versions = ["a", "b"]

[[class]]
java = "fx.Widget"
members = ["noSuchThing", "isReady"]
"#;
    let spec_path = out.join("spec.toml");
    std::fs::write(&spec_path, spec).expect("spec");
    let err = generate(&spec_path, &versions(&["a", "b"]), &out.join("bindings"))
        .err()
        .expect("generation fails");
    assert!(format!("{err:#}").contains("noSuchThing"), "{err:#}");

    let spec = spec.replace("\"noSuchThing\"", "\"noSuchThing when b\"");
    std::fs::write(&spec_path, spec).expect("spec");
    let err = generate(&spec_path, &versions(&["a"]), &out.join("bindings"));
    assert!(
        err.is_ok(),
        "a conditional member of another version is skipped: {:?}",
        err.err()
    );
    let err = generate(&spec_path, &versions(&["b"]), &out.join("bindings"))
        .err()
        .expect("in b the member is wanted and missing");
    assert!(format!("{err:#}").contains("noSuchThing"), "{err:#}");
}

#[test]
fn a_condition_needs_versions() {
    assert!(blazon::spec::parse_member("x when , ").is_err());
    let m = blazon::spec::parse_member("put(I) as put_at? when 9.0.1, 9.2.0").expect("member");
    assert_eq!(m.versions, ["9.0.1", "9.2.0"]);
    assert_eq!(m.text, "put(I) as put_at?");
    assert!(m.nullable && m.rust.as_deref() == Some("put_at"));
}

/// The two `BuildEventServiceGrpcClient` constructors of Bazel 9.0.1 and 9.2.0.
#[test]
fn the_grpc_client_constructors_differ_across_the_live_releases() -> anyhow::Result<()> {
    let Some(home) = std::env::var_os("HOME") else {
        return Ok(());
    };
    let cache = PathBuf::from(home).join("Library/Caches/buildx/bazel");
    let jars = ["9.0.1", "9.2.0"].map(|v| cache.join(v).join("A-server.jar"));
    if !jars.iter().all(|j| j.exists()) {
        return Ok(());
    }
    let out = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join("versions-live");
    std::fs::create_dir_all(&out)?;
    let spec_path = out.join("spec.toml");
    std::fs::write(
        &spec_path,
        r#"
versions = ["9.0.1", "9.2.0"]

[[class]]
java = "io.grpc.ManagedChannel"
members = []

[[class]]
java = "io.grpc.CallCredentials"
members = []

[[class]]
java = "io.grpc.ClientInterceptor"
members = []

[[class]]
java = "com.google.devtools.build.lib.buildeventservice.client.BuildEventServiceGrpcClient"
members = [
    "<init>(Lio/grpc/ManagedChannel;Lio/grpc/CallCredentials;Lio/grpc/ClientInterceptor;Ljava/lang/String;Ljava/util/UUID;) when 9.0.1",
    "<init>(Lio/grpc/ManagedChannel;Lio/grpc/CallCredentials;Lio/grpc/ClientInterceptor;) when 9.2.0",
]
"#,
    )?;
    let report = generate(
        &spec_path,
        &[
            Version {
                label: "9.0.1".into(),
                jar: jars[0].clone(),
            },
            Version {
                label: "9.2.0".into(),
                jar: jars[1].clone(),
            },
        ],
        &out.join("bindings"),
    )?;
    let shape = report.shape.expect("report");
    assert!(shape.contains("only in 9.0.1  "), "{shape}");
    assert!(shape.contains("only in 9.2.0  "), "{shape}");
    let old = std::fs::read_to_string(out.join("bindings/v9_0_1.rs"))?;
    let new = std::fs::read_to_string(out.join("bindings/v9_2_0.rs"))?;
    assert!(
        old.contains("p4: &impl blazon_rt::IsA<UUID<'static>>"),
        "9.0.1 takes the UUID"
    );
    assert!(!new.contains("p3: &str"), "9.2.0 does not");
    Ok(())
}
