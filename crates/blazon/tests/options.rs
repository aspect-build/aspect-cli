//! The option schema over the fixture's option classes (`tests/fixture/src`): every
//! kind of value, every way an option is included or not, and the emitted table.

use blazon::classfile::Classes;
use blazon::jar::Jar;
use blazon::options::{OptKind, Schema, schema};
use blazon::spec::OptionsSpec;
use std::path::PathBuf;

fn classes() -> Classes {
    let jar = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixture/fixture.jar");
    Classes::new(Jar::open(&jar).expect("fixture jar"))
}

fn rules(text: &str) -> OptionsSpec {
    toml::from_str(text).expect("options spec")
}

const RULES: &str = r#"
fragments = true
include_effect_tags = ["LOADING_AND_ANALYSIS", "BUILD_FILE_SEMANTICS"]
exclude_effect_tags = ["EXECUTION", "NO_OP", "TERMINAL_OUTPUT", "HOST_MACHINE_RESOURCE_OPTIMIZATIONS"]
exclude_classes = ["fx.opts.RemoteStuff"]
include = ["forced_in"]
exclude = ["forced_out"]
"#;

fn names(s: &Schema) -> Vec<&str> {
    s.options.iter().map(|o| o.name.as_str()).collect()
}

fn kind<'a>(s: &'a Schema, name: &str) -> &'a OptKind {
    &s.options.iter().find(|o| o.name == name).expect(name).kind
}

#[test]
fn every_options_class_is_found() {
    let mut c = classes();
    let all = blazon::options::discover(&mut c).expect("discover");
    let found: Vec<&str> = all.iter().map(|(n, _)| n.as_str()).collect();
    assert_eq!(
        found,
        [
            "com/google/devtools/build/lib/analysis/config/FragmentOptions",
            "fx/opts/Dup",
            "fx/opts/Fragment",
            "fx/opts/Loading",
            "fx/opts/RemoteStuff"
        ]
    );
    let fragment = &all[2].1;
    assert_eq!(fragment.len(), 21, "only the fields with @Option");
}

#[test]
fn inclusion_follows_the_rules_in_order() {
    let s = schema(&mut classes(), &rules(RULES), "fixture").expect("schema");
    let included = names(&s);
    // Every option of a fragment, whatever its tags, except the internal one.
    for name in [
        "big", "ratio", "timeout", "jobs", "old_flag", "implied", "flag", "count", "text",
    ] {
        assert!(included.contains(&name), "{name} is in a fragment");
    }
    assert!(!included.contains(&"hidden_internal"), "INTERNAL is out");
    // Outside a fragment, by effect tag, unless the tags are only excluded ones.
    assert!(included.contains(&"by_tag"));
    assert!(
        included.contains(&"by_tag_too"),
        "one included tag is enough"
    );
    assert!(!included.contains(&"unrelated"), "only EXECUTION");
    // Named options beat tags; excluded classes beat tags.
    assert!(included.contains(&"forced_in"));
    assert!(!included.contains(&"forced_out"));
    assert!(!included.contains(&"remote_thing"));
    assert_eq!(s.discovered, 21 + 5 + 1 + 1);
}

#[test]
fn rules_can_be_narrower() {
    let narrow = rules(
        r#"
include_effect_tags = ["LOADING_AND_ANALYSIS"]
exclude_effect_tags = ["EXECUTION"]
exclude_metadata_tags = ["INTERNAL", "DEPRECATED"]
include_classes = ["fx.opts.Loading"]
"#,
    );
    let s = schema(&mut classes(), &narrow, "fixture").expect("schema");
    let included = names(&s);
    assert!(
        !included.contains(&"old_flag"),
        "DEPRECATED is excluded here"
    );
    assert!(
        !included.contains(&"flag"),
        "no fragments rule, and AFFECTS_OUTPUTS is not an included tag"
    );
    assert!(included.contains(&"count"), "LOADING_AND_ANALYSIS");
    for name in [
        "by_tag",
        "by_tag_too",
        "unrelated",
        "forced_in",
        "forced_out",
    ] {
        assert!(included.contains(&name), "{name} is in an included class");
    }
    assert!(
        included.contains(&"remote_thing"),
        "nothing excludes this class here"
    );
}

#[test]
fn class_patterns_end_in_a_star() {
    let r = rules(
        r#"
include_effect_tags = ["LOADING_AND_ANALYSIS"]
exclude_classes = ["fx.opts.*"]
"#,
    );
    let s = schema(&mut classes(), &r, "fixture").expect("schema");
    assert!(s.options.is_empty());
}

#[test]
fn values_are_classified_by_type_and_converter() {
    let s = schema(&mut classes(), &rules(RULES), "fixture").expect("schema");
    assert_eq!(kind(&s, "flag"), &OptKind::Bool);
    assert_eq!(kind(&s, "count"), &OptKind::Int);
    assert_eq!(kind(&s, "big"), &OptKind::Long);
    assert_eq!(kind(&s, "ratio"), &OptKind::Double);
    assert_eq!(kind(&s, "text"), &OptKind::String);
    assert_eq!(
        kind(&s, "mode"),
        &OptKind::Enum(vec!["FASTBUILD".into(), "DBG".into(), "OPT".into()]),
        "an enum type with a converter blazon does not know is still an enum"
    );
    assert_eq!(kind(&s, "kind"), kind(&s, "mode"));
    assert_eq!(kind(&s, "target"), &OptKind::Label);
    assert_eq!(
        kind(&s, "targets"),
        &OptKind::List(Box::new(OptKind::Label))
    );
    assert_eq!(kind(&s, "dir"), &OptKind::Path);
    assert_eq!(kind(&s, "timeout"), &OptKind::Duration);
    assert_eq!(kind(&s, "tag"), &OptKind::List(Box::new(OptKind::String)));
    assert_eq!(kind(&s, "csv"), &OptKind::List(Box::new(OptKind::String)));
    assert_eq!(kind(&s, "env"), &OptKind::Dict(Box::new(OptKind::String)));
    assert_eq!(kind(&s, "vars"), &OptKind::Dict(Box::new(OptKind::String)));
    assert_eq!(
        kind(&s, "limits"),
        &OptKind::Dict(Box::new(OptKind::List(Box::new(OptKind::String))))
    );
    assert_eq!(kind(&s, "no_value"), &OptKind::Void);
    assert_eq!(
        kind(&s, "jobs"),
        &OptKind::Opaque("fx.opts.Jobs".into()),
        "an int with a converter blazon does not know is the converter's to read"
    );
}

#[test]
fn annotation_fields_are_carried() {
    let s = schema(&mut classes(), &rules(RULES), "fixture").expect("schema");
    let find = |n: &str| s.options.iter().find(|o| o.name == n).expect(n);
    let flag = find("flag");
    assert_eq!(flag.abbrev, Some('f'));
    assert_eq!(flag.help, "A boolean.");
    assert_eq!(flag.default, "false");
    assert_eq!(flag.category, "TOOLCHAIN");
    assert_eq!(flag.effect_tags, ["AFFECTS_OUTPUTS"]);
    assert_eq!(flag.class, "fx.opts.Fragment");
    assert_eq!(flag.field, "flag");
    assert!(flag.fragment && !find("by_tag").fragment);
    let old = find("old_flag");
    assert!(old.deprecated);
    assert_eq!(old.deprecation_warning, "Use `flag`.");
    assert_eq!(old.old_names, ["older_flag"]);
    assert_eq!(old.metadata_tags, ["DEPRECATED", "EXPERIMENTAL"]);
    let no_value = find("no_value");
    assert!(no_value.expansion);
    assert_eq!(no_value.expands_to, ["--flag", "--count=1"]);
    assert_eq!(find("implied").implicit_requirements, ["--flag=true"]);
    assert!(find("tag").allows_multiple && !find("csv").allows_multiple);
    assert!(find("count").help.is_empty());
}

#[test]
fn the_summary_names_the_gaps() {
    let s = schema(&mut classes(), &rules(RULES), "fixture").expect("schema");
    let text = s.summary();
    assert!(text.contains("Opaque"), "{text}");
    assert!(text.contains("jobs (Opaque(fx.opts.Jobs))"), "{text}");
    assert!(
        text.contains("included options with no help text"),
        "{text}"
    );
}

#[test]
fn a_name_two_classes_declare_is_an_error() {
    let r = rules(
        r#"
include_classes = ["fx.opts.Loading", "fx.opts.Dup"]
"#,
    );
    let err = schema(&mut classes(), &r, "fixture")
        .unwrap_err()
        .to_string();
    assert!(err.contains("by_tag"), "{err}");
    assert!(
        err.contains("fx.opts.Loading") && err.contains("fx.opts.Dup"),
        "{err}"
    );
    assert!(err.contains("Bazel fixture"), "{err}");
}

fn dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests")
}

/// The emitted table for the fixture, `BLAZON_BLESS=1` to rewrite the snapshot.
#[test]
fn the_emitted_table_matches_its_snapshot() -> anyhow::Result<()> {
    let out = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join("options-snapshot");
    std::fs::create_dir_all(&out)?;
    let spec = out.join("spec.toml");
    std::fs::write(
        &spec,
        format!("versions = [\"fixture\"]\n\n[options]\n{RULES}"),
    )?;
    blazon::generate(
        &spec,
        &[blazon::Version {
            label: "fixture".into(),
            jar: dir().join("fixture/fixture.jar"),
        }],
        &out.join("bindings"),
    )?;
    let got = std::fs::read_to_string(out.join("bindings/vfixture.rs"))?;
    let want_path = dir().join("snapshots/options.rs.txt");
    if std::env::var_os("BLAZON_BLESS").is_some() {
        std::fs::write(&want_path, &got)?;
    }
    let want = std::fs::read_to_string(&want_path).unwrap_or_default();
    assert!(
        got == want,
        "generated code differs from {}; rerun with BLAZON_BLESS=1 to accept",
        want_path.display()
    );
    let mod_rs = std::fs::read_to_string(out.join("bindings/mod.rs"))?;
    assert!(mod_rs.contains("pub fn options(version: &str)"), "{mod_rs}");
    Ok(())
}
