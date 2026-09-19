//! Where `aspect setup bazelrc` puts the rc, and what it can write with nothing
//! configured.
//!
//! `--home` writes the machine's rc under the home directory instead of the
//! checkout's. The layout is the same one the checkout gets — the generated rc
//! at `.aspect/bazelrc`, `try-import`ed from the `.bazelrc` beside it, the
//! execution log under `.aspect/generated/` — only rooted at `$HOME`, so an
//! ephemeral CI runner throws it away with the job and nothing can be committed
//! by accident. Two things differ from a checkout and are what these tests pin:
//! no workspace is needed to write it, and `.aspect/generated/` gets no
//! `.gitignore`, a home directory not being a checkout.
//!
//! `setup_bazelrc_unconfigured` covers the other shape: no `ASPECT_WORKFLOWS_*`,
//! so the rc carries the opt-in deployment sections, written from a home
//! directory that has never been logged in to. Aspect Cloud is among those
//! sections there, which is the second thing pinned here.
//!
//! Every test runs against a temporary `$HOME`, since that is what decides where
//! the rc goes and which deployments are on record; the credential store is
//! redirected as well so none can reach a keyring.

mod common;

use common::aspect_cli;
use std::path::Path;
use std::process::{Command, Output};

/// Run `aspect setup bazelrc <args>` with `home` as the home directory and a cwd
/// of `cwd`, against an environment that names the endpoints itself.
fn setup_bazelrc(home: &Path, cwd: &Path, args: &[&str]) -> Output {
    Command::new(aspect_cli())
        .arg("setup")
        .arg("bazelrc")
        .args(args)
        .current_dir(cwd)
        .env("HOME", home)
        .env("ASPECT_CREDENTIALS_FILE", home.join("credentials.json"))
        .env("ASPECT_WORKFLOWS_REMOTE_CACHE", "grpcs://cache.example:443")
        .env("ASPECT_WORKFLOWS_BES_BACKEND", "grpcs://bes.example:443")
        .env_remove("ASPECT_AUTH_PROFILE")
        .env_remove("ASPECT_API_TOKEN")
        .env_remove("ASPECT_DEBUG")
        .env_remove("ASPECT_WORKFLOWS_RUNNER")
        .output()
        .expect("running `aspect setup bazelrc`")
}

fn assert_success(output: &Output) {
    assert_eq!(
        output.status.code(),
        Some(0),
        "expected success\n--- stderr ---\n{}",
        String::from_utf8_lossy(&output.stderr)
    );
}

fn read(path: &Path) -> String {
    std::fs::read_to_string(path).unwrap_or_else(|e| panic!("reading {}: {e}", path.display()))
}

#[test]
fn home_mode_writes_the_rc_pair_under_the_home_directory() {
    let home = tempfile::tempdir().expect("temp home");
    let cwd = tempfile::tempdir().expect("temp cwd");
    let home = home.path();

    assert_success(&setup_bazelrc(home, cwd.path(), &["--home"]));

    // The generated rc, and the `.bazelrc` Bazel already reads there importing
    // it by the path it was written to — `%workspace%` would be wrong for a
    // file outside any checkout.
    let generated = home.join(".aspect/bazelrc");
    let rc = read(&generated);
    assert!(
        rc.contains("common --config=aspect-cache")
            && rc.contains("--bes_backend=grpcs://bes.example:443"),
        "the env's endpoints should be enabled in:\n{rc}"
    );
    assert_eq!(
        read(&home.join(".bazelrc")),
        format!(
            "# Aspect's rc for vanilla `bazel`, maintained by `aspect setup bazelrc`.\ntry-import {}\n",
            generated.display()
        )
    );

    // The execution log lands in the home directory's generated dir, created
    // because Bazel will not create the log's parent, and bare: a `.gitignore`
    // there would ignore nothing.
    assert!(
        rc.contains(&format!(
            "--execution_log_compact_file={}",
            home.join(".aspect/generated/exec.log.zstd").display()
        )),
        "the log should sit under the home base in:\n{rc}"
    );
    assert!(home.join(".aspect/generated").is_dir());
    assert!(!home.join(".aspect/generated/.gitignore").exists());

    // No Bazel workspace anywhere above the cwd, and none needed.
    assert_eq!(
        std::fs::read_dir(cwd.path()).expect("cwd").count(),
        0,
        "home mode must leave the working directory alone"
    );
}

#[test]
fn home_mode_is_idempotent() {
    let home = tempfile::tempdir().expect("temp home");
    let cwd = tempfile::tempdir().expect("temp cwd");

    assert_success(&setup_bazelrc(home.path(), cwd.path(), &["--home"]));
    let first = read(&home.path().join(".aspect/bazelrc"));
    assert_success(&setup_bazelrc(home.path(), cwd.path(), &["--home"]));

    assert_eq!(first, read(&home.path().join(".aspect/bazelrc")));
    assert_eq!(
        read(&home.path().join(".bazelrc"))
            .matches("try-import")
            .count(),
        1,
        "the import is added once, not on every run"
    );
}

/// Run without the `ASPECT_WORKFLOWS_*` endpoints, so the rc carries the opt-in
/// deployment sections rather than taking its endpoints from the environment.
fn setup_bazelrc_unconfigured(home: &Path, cwd: &Path, args: &[&str]) -> Output {
    Command::new(aspect_cli())
        .args(["setup", "bazelrc"])
        .args(args)
        .current_dir(cwd)
        .env("HOME", home)
        .env("ASPECT_CREDENTIALS_FILE", home.join("credentials.json"))
        .env_remove("ASPECT_AUTH_PROFILE")
        .env_remove("ASPECT_API_TOKEN")
        .env_remove("ASPECT_DEBUG")
        .output()
        .expect("running `aspect setup bazelrc`")
}

/// Aspect Cloud is among the sections on a machine that has never logged in,
/// because the built-in entry states its own endpoints.
#[test]
fn a_machine_that_has_never_logged_in_still_gets_an_aspect_cloud_section() {
    let home = tempfile::tempdir().expect("temp home");
    let workspace = tempfile::tempdir().expect("temp workspace");
    std::fs::write(workspace.path().join("MODULE.bazel"), "").expect("MODULE.bazel");

    let output = setup_bazelrc_unconfigured(home.path(), workspace.path(), &[]);
    assert_success(&output);
    assert!(
        !home.path().join(".aspect/config.json").exists(),
        "the test must exercise the no-config path"
    );

    let rc = read(&workspace.path().join(".aspect/bazelrc"));
    assert!(
        rc.contains("common:aspect-cloud --remote_cache=grpcs://cache.aspect.build")
            && rc.contains("common:aspect-cloud --bes_backend=grpcs://bes.aspect.build"),
        "expected Aspect Cloud's seeded endpoints in:\n{rc}"
    );
    // No remote executor is advertised, so there is no section to opt into one.
    assert!(!rc.contains("aspect-cloud-exec"), "in:\n{rc}");
}

/// The CI shape `--home` exists for: a stock runner whose environment names no
/// endpoint. The rc is this machine's, so unlike a checkout's it *enables* the
/// tuning that presumes no endpoint — a plain `bazel build` picks that up with no
/// `--config` — while the endpoints stay behind the deployment sections.
#[test]
fn home_mode_enables_the_endpointless_tuning_and_says_the_files_are_this_machines() {
    let home = tempfile::tempdir().expect("temp home");
    let cwd = tempfile::tempdir().expect("temp cwd");

    let output = setup_bazelrc_unconfigured(home.path(), cwd.path(), &["--home"]);
    assert_success(&output);

    let rc = read(&home.path().join(".aspect/bazelrc"));
    assert!(
        rc.contains("common --config=aspect-common"),
        "the endpointless tuning should be on for every call in:\n{rc}"
    );
    assert!(
        !rc.contains("common --config=aspect-cloud"),
        "choosing a deployment stays the caller's in:\n{rc}"
    );
    assert!(
        rc.contains("common:aspect-cloud --remote_cache="),
        "in:\n{rc}"
    );

    // The report is console output, so it goes to stderr like every other
    // human-facing line the CLI writes.
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("These are this machine's, outside the checkout."),
        "the report must not tell CI to commit them:\n{stderr}"
    );
    assert!(
        stderr.contains("~/.aspect/bazelrc"),
        "paths should read as the home directory's:\n{stderr}"
    );
}

#[test]
fn home_and_output_are_refused_together() {
    let home = tempfile::tempdir().expect("temp home");
    let cwd = tempfile::tempdir().expect("temp cwd");
    let output = setup_bazelrc(
        home.path(),
        cwd.path(),
        &["--home", "--output=elsewhere.bazelrc"],
    );

    assert_eq!(output.status.code(), Some(1));
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("--home and --output both choose where the rc goes"),
        "expected the refusal in:\n{stderr}"
    );
    assert!(!home.path().join(".aspect/bazelrc").exists());
}
