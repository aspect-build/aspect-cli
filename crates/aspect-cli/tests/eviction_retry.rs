//! A build a remote cache eviction fails is retried by the CLI, as an
//! invocation of its own, with Bazel's own eviction retries turned off.
//!
//! Each case runs `aspect build` in a fresh workspace against the `basil`
//! fake, whose `cache_evicted_once` scenario exits 39 on the first invocation
//! and succeeds on every later one. `BASIL_ARGV_LOG` records the command line
//! of each, which is what says how often Bazel ran and with what.

mod common;

use std::path::{Path, PathBuf};
use std::process::{Command, Output, Stdio};

use common::{aspect_cli, base_env, basil_bin};

const EVICTION_RETRIES: &str = "--experimental_remote_cache_eviction_retries";

/// A workspace whose builds run the `cache_evicted_once` scenario, with the
/// hooks reporting each attempt's exit code and the final one.
fn workspace(bazelrc: &str) -> tempfile::TempDir {
    let dir = tempfile::tempdir().expect("temp dir");
    let root = dir.path();
    std::fs::write(root.join("MODULE.bazel"), "").expect("MODULE.bazel");
    std::fs::write(root.join("MODULE.aspect"), "").expect("MODULE.aspect");
    std::fs::write(root.join(".bazelrc"), bazelrc).expect(".bazelrc");
    std::fs::create_dir(root.join(".aspect")).expect(".aspect");
    std::fs::write(
        root.join(".aspect").join("config.axl"),
        r#"load("@aspect//traits.axl", "BazelTrait")

def config(ctx: ConfigContext):
    t = ctx.traits[BazelTrait]
    t.extra_flags.append("--scenario=cache_evicted_once")
    t.bazel_attempt_end.append(lambda ctx, code: print("attempt_end=%d" % code))
    t.build_end.append(lambda ctx, code: print("build_end=%d" % code))
"#,
    )
    .expect("config.axl");
    dir
}

/// Runs `aspect build //... <args>` in `root`, returning its output and the
/// command lines of the Bazel builds it ran.
fn build(root: &Path, args: &[&str]) -> (Output, Vec<String>) {
    let argv = root.join("argv.log");
    let output = Command::new(aspect_cli())
        .args(["build", "//..."])
        .args(args)
        .current_dir(root)
        .env_clear()
        .envs(base_env())
        .env("BAZEL_REAL", basil_bin())
        .env("BASIL_ARGV_LOG", &argv)
        .env("BASIL_EVICTED_ONCE", root.join("evicted"))
        .env("ASPECT_HISTORY_DIR", root.join("history"))
        .env("ASPECT_CREDENTIALS_FILE", root.join("credentials.json"))
        .env("HOME", root.join("home"))
        .env("DO_NOT_TRACK", "1")
        .stdin(Stdio::null())
        .output()
        .expect("running aspect build");
    let builds = std::fs::read_to_string(&argv)
        .unwrap_or_default()
        .lines()
        .filter(|l| l.split_whitespace().any(|a| a == "build"))
        .map(str::to_owned)
        .collect();
    (output, builds)
}

/// The `bazel/<n>` folders of the one task invocation in the history.
fn bazel_folders(history: &Path) -> Vec<PathBuf> {
    let invocations: Vec<PathBuf> = std::fs::read_dir(history)
        .expect("the history")
        .map(|e| e.expect("entry").path())
        .filter(|p| !p.file_name().unwrap().to_string_lossy().starts_with('.'))
        .collect();
    assert_eq!(invocations.len(), 1, "one task invocation: {invocations:?}");
    let mut folders: Vec<PathBuf> = std::fs::read_dir(invocations[0].join("bazel"))
        .expect("its bazel folder")
        .map(|e| e.expect("entry").path())
        .collect();
    folders.sort();
    folders
}

/// An eviction is retried as a second Bazel invocation, both with Bazel's own
/// eviction retries off, each in a history folder of its own.
#[test]
fn an_eviction_is_retried_as_an_invocation_of_its_own() {
    let dir = workspace("");
    let (output, builds) = build(dir.path(), &[]);
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert_eq!(output.status.code(), Some(0), "{stderr}");
    assert_eq!(builds.len(), 2, "one retry:\n{stderr}");
    let off = format!("{EVICTION_RETRIES}=0");
    for line in &builds {
        assert!(line.contains(&off), "Bazel's retries off: {line}");
    }
    let attempts: Vec<&str> = stderr
        .lines()
        .filter(|l| l.contains("attempt_end="))
        .collect();
    assert!(
        attempts.len() == 2 && attempts[0].contains("=39") && attempts[1].contains("=0"),
        "{stderr}"
    );
    assert!(stderr.contains("build_end=0"), "{stderr}");
    assert_eq!(
        bazel_folders(&dir.path().join("history")).len(),
        2,
        "a history folder per attempt"
    );
}

/// With no retry to hand them to, Bazel's own eviction retries are left as
/// they are, and the eviction is the task's exit.
#[test]
fn one_attempt_leaves_bazels_retries_alone() {
    let dir = workspace("");
    let (output, builds) = build(dir.path(), &["--bazel-retry-attempts=1"]);
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert_eq!(output.status.code(), Some(39), "{stderr}");
    assert_eq!(builds.len(), 1, "{stderr}");
    assert!(!builds[0].contains(EVICTION_RETRIES), "{}", builds[0]);
}

/// A setting the user's rc makes is theirs: the CLI adds none of its own, and
/// still retries what Bazel gives up on.
#[test]
fn an_rc_setting_is_kept() {
    let dir = workspace(&format!("build {EVICTION_RETRIES}=2\n"));
    let (output, builds) = build(dir.path(), &[]);
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert_eq!(output.status.code(), Some(0), "{stderr}");
    assert_eq!(builds.len(), 2, "{stderr}");
    let off = format!("{EVICTION_RETRIES}=0");
    for line in &builds {
        assert!(!line.contains(&off), "the rc's setting stands: {line}");
    }
}
