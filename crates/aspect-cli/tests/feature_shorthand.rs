//! A feature's bare `--<name>` flag turns it on or off like `--<name>:enabled`,
//! and no feature's shorthand takes a flag something else already owns.
//!
//! Spawns the real CLI in scratch workspaces: one with only the builtins, one
//! with a copy of this repo's `MODULE.aspect` and `.aspect/`, and one with a
//! probe feature of its own.

mod common;

use common::{aspect_cli, base_env};
use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};
use std::process::{Command, Output};

/// Features whose shorthand is skipped, and why. A new collision fails
/// [`no_feature_shorthand_collides`]: rename the feature or flag, or add it
/// here with its reason.
const SKIPPED: &[(&str, &str)] = &[(
    "deployment",
    "its own `--deployment` arg (and the auth and mcp tasks' `--deployment`) owns the flag",
)];

const VALUE_FLAGS: &str = include_str!("../src/builtins/aspect/bazel/value_flags.axl");

fn aspect(dir: &Path, args: &[&str]) -> Output {
    Command::new(aspect_cli())
        .args(args)
        .current_dir(dir)
        .env_clear()
        .envs(base_env())
        .env("ASPECT_CLI_CACHE", dir.join("cache"))
        .env("HOME", dir.join("home"))
        .output()
        .unwrap_or_else(|e| panic!("running `aspect {}`: {e}", args.join(" ")))
}

fn json(output: Output) -> serde_json::Value {
    assert!(output.status.success(), "{output:?}");
    serde_json::from_slice(&output.stdout).expect("describe prints JSON")
}

/// The directory holding this repo's `MODULE.aspect` and `.aspect/`: from
/// Bazel's runfiles when it sets `ASPECT_REPO_MODULE`, else the cargo
/// workspace root (read at run time: `env!` would embed the build path).
fn repo_config_root() -> PathBuf {
    match std::env::var("ASPECT_REPO_MODULE") {
        Ok(p) => Path::new(&p)
            .parent()
            .expect("MODULE.aspect parent")
            .to_path_buf(),
        Err(_) => Path::new(
            &std::env::var("CARGO_MANIFEST_DIR")
                .expect("set ASPECT_REPO_MODULE or run under cargo"),
        )
        .join("../.."),
    }
}

/// Copy `from` into `to`, following symlinks, as runfiles are.
fn copy_tree(from: &Path, to: &Path) {
    std::fs::create_dir_all(to).expect("mkdir");
    for entry in std::fs::read_dir(from).expect("read_dir") {
        let entry = entry.expect("entry");
        let target = to.join(entry.file_name());
        if std::fs::metadata(entry.path()).expect("metadata").is_dir() {
            copy_tree(&entry.path(), &target);
        } else {
            std::fs::copy(entry.path(), &target).expect("copy");
        }
    }
}

/// Each feature's `shorthand` and `shorthand_unavailable_on`, as `aspect
/// describe` reports them in `dir`.
fn shorthands(dir: &Path) -> BTreeMap<String, (Option<String>, Vec<String>)> {
    let names: Vec<String> = json(aspect(dir, &["describe"]))["features"]
        .as_array()
        .expect("features")
        .iter()
        .map(|f| f.as_str().expect("name").to_owned())
        .collect();
    std::thread::scope(|scope| {
        let handles: Vec<_> = names
            .iter()
            .map(|name| {
                scope.spawn(move || {
                    let doc = json(aspect(dir, &["describe", &format!("feature {name}")]));
                    let unavailable = doc["shorthand_unavailable_on"]
                        .as_array()
                        .expect("shorthand_unavailable_on")
                        .iter()
                        .map(|c| c.as_str().expect("command").to_owned())
                        .collect();
                    (
                        name.clone(),
                        (doc["shorthand"].as_str().map(str::to_owned), unavailable),
                    )
                })
            })
            .collect();
        handles
            .into_iter()
            .map(|h| h.join().expect("describe"))
            .collect()
    })
}

#[test]
fn no_feature_shorthand_collides() {
    let builtins = tempfile::tempdir().expect("temp dir");
    std::fs::write(builtins.path().join("MODULE.aspect"), "").expect("MODULE.aspect");

    let repo = tempfile::tempdir().expect("temp dir");
    let source = repo_config_root();
    std::fs::copy(
        source.join("MODULE.aspect"),
        repo.path().join("MODULE.aspect"),
    )
    .expect("MODULE.aspect");
    copy_tree(&source.join(".aspect"), &repo.path().join(".aspect"));

    let bazel_value_flags: BTreeSet<&str> = VALUE_FLAGS
        .lines()
        .filter_map(|l| l.trim().strip_prefix('"')?.split('"').next())
        .filter(|f| f.starts_with("--"))
        .collect();
    assert!(
        bazel_value_flags.contains("--jobs"),
        "value_flags.axl parsed"
    );

    let want_skipped: BTreeSet<&str> = SKIPPED.iter().map(|(name, _)| *name).collect();
    for dir in [builtins.path(), repo.path()] {
        let found = shorthands(dir);
        assert!(found.contains_key("tips"), "builtins loaded in {dir:?}");
        let skipped: BTreeSet<&str> = found
            .iter()
            .filter(|(_, (flag, _))| flag.is_none())
            .map(|(name, _)| name.as_str())
            .collect();
        assert_eq!(skipped, want_skipped, "skipped shorthands in {dir:?}");
        for (name, (flag, unavailable)) in &found {
            assert!(
                unavailable.is_empty(),
                "--{name} is a task's own flag on {unavailable:?}"
            );
            if let Some(flag) = flag {
                assert_eq!(flag, &format!("--{name}"));
                assert!(
                    !bazel_value_flags.contains(flag.as_str()),
                    "--{name} is a Bazel flag"
                );
            }
        }
    }
}

const PROBE: &str = r#"
def _impl(ctx):
    print("probe ran")

Probe = feature(
    implementation = _impl,
    summary = "Prints when it runs.",
    enabled = False,
)
"#;

const TASK: &str = r#"
def _impl(ctx):
    return 0

check = task(
    implementation = _impl,
    summary = "Do nothing.",
    args = {},
)
"#;

/// Run `aspect check <args>` with the off-by-default `Probe` feature, under a
/// `config.axl` whose `config(ctx)` runs `config_body`.
fn probe(config_body: &str, args: &[&str]) -> Output {
    let dir = tempfile::tempdir().expect("temp dir");
    std::fs::write(
        dir.path().join("MODULE.aspect"),
        "use_feature(\".aspect/probe.axl\", \"Probe\")\n",
    )
    .expect("MODULE.aspect");
    let aspect_dir = dir.path().join(".aspect");
    std::fs::create_dir(&aspect_dir).expect(".aspect");
    std::fs::write(aspect_dir.join("probe.axl"), PROBE).expect("probe.axl");
    std::fs::write(aspect_dir.join("tasks.axl"), TASK).expect("tasks.axl");
    std::fs::write(
        aspect_dir.join("config.axl"),
        format!(
            "load(\"./probe.axl\", \"Probe\")\n\ndef config(ctx):\n    pass\n    {config_body}\n"
        ),
    )
    .expect("config.axl");
    let mut argv = vec!["check"];
    argv.extend_from_slice(args);
    aspect(dir.path(), &argv)
}

fn ran(output: &Output) -> bool {
    assert!(output.status.success(), "{output:?}");
    String::from_utf8_lossy(&output.stderr).contains("probe ran")
}

#[test]
fn shorthand_turns_a_feature_on_and_off() {
    assert!(!ran(&probe("", &[])));
    assert!(ran(&probe("", &["--probe"])));
    assert!(ran(&probe("", &["--probe=true"])));
    assert!(ran(&probe("", &["--probe:enabled"])));

    let on = "ctx.features[Probe].enabled = True";
    assert!(ran(&probe(on, &[])));
    assert!(!ran(&probe(on, &["--probe=false"])));
    assert!(!ran(&probe(
        on,
        &["--probe=false", "--probe:enabled=false"]
    )));
}

#[test]
fn shorthand_conflicting_with_enabled_fails_naming_both() {
    let output = probe("", &["--probe", "--probe:enabled=false"]);
    assert!(!output.status.success(), "{output:?}");
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("--probe=true conflicts with --probe:enabled=false"),
        "{stderr}"
    );
}

#[test]
fn feature_help_shows_the_shorthand_beside_enabled() {
    let dir = tempfile::tempdir().expect("temp dir");
    std::fs::write(dir.path().join("MODULE.aspect"), "").expect("MODULE.aspect");
    let output = aspect(dir.path(), &["feature", "tips"]);
    assert!(output.status.success(), "{output:?}");
    let help = String::from_utf8_lossy(&output.stdout);
    assert!(
        help.contains("\n      --tips, --tips:enabled[=<tips:enabled>]\n"),
        "{help}"
    );
    let deployment = aspect(dir.path(), &["feature", "deployment"]);
    let help = String::from_utf8_lossy(&deployment.stdout);
    assert!(
        help.contains("\n      --deployment:enabled[=<deployment:enabled>]\n"),
        "{help}"
    );
}
