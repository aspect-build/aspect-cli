//! The Buildkite annotation surface, driven end to end: a fixture task emits
//! `phases.update`s under `BUILDKITE=true` with a fake `buildkite-agent` on
//! `PATH` that records every `annotate` body it receives.
//!
//! An update with no `kind` is the lifecycle's "no template" case and must be
//! skipped without a trace; an update that carries the surface's kind must
//! re-render the annotation with its progress text.

#![cfg(unix)]

mod common;

use std::os::unix::fs::PermissionsExt;
use std::process::Command;

use common::{aspect_cli, base_env};

const FIXTURE: &str = r#"
load("@aspect//private/lib/bazel_results.axl", "init_data")
load("@aspect//private/lib/lifecycle.axl", "phases")

def _impl(ctx):
    ph = phases.new(ctx, init_data())
    ph.setup_surface("//probe", "bazel_results")
    ph.update("running", phases.ProgressStyles(body = "Untemplated marker"))
    ph.update(
        "running",
        phases.ProgressStyles(body = "Running //probe..."),
        kind = "bazel_results",
        phase = phases.Phase(name = "run", description = "Run target", emoji = "🚀"),
    )
    return ph.update("passed", "Done", kind = "bazel_results", final = True, conclusion = "Done")

probe = task(summary = "Test fixture.", implementation = _impl)
"#;

/// Appends each `annotate` body to `$AGENT_LOG`, separated by a marker line.
const FAKE_AGENT: &str = r#"#!/bin/sh
if [ "$1" = "annotate" ]; then
  cat >> "$AGENT_LOG"
  echo "--- end annotate ---" >> "$AGENT_LOG"
fi
exit 0
"#;

/// Runs the fixture with trace logging; returns (stderr, annotate bodies).
fn run_probe() -> (String, Vec<String>) {
    let dir = tempfile::tempdir().expect("temp dir");
    let root = dir.path();
    std::fs::write(root.join("MODULE.bazel"), "").expect("MODULE.bazel");
    std::fs::create_dir(root.join(".aspect")).expect(".aspect");
    std::fs::write(root.join(".aspect/fixture.axl"), FIXTURE).expect("fixture");

    let bin = root.join("bin");
    std::fs::create_dir(&bin).expect("bin");
    let agent = bin.join("buildkite-agent");
    std::fs::write(&agent, FAKE_AGENT).expect("fake agent");
    std::fs::set_permissions(&agent, std::fs::Permissions::from_mode(0o755)).expect("chmod");
    let log = root.join("annotate.log");

    let path = format!(
        "{}:{}",
        bin.display(),
        std::env::var("PATH").unwrap_or_default()
    );
    let out = Command::new(aspect_cli())
        .arg("probe")
        .current_dir(root)
        .env_clear()
        .envs(base_env())
        .env("PATH", path)
        .env("AGENT_LOG", &log)
        .env("BUILDKITE", "true")
        .env("ASPECT_DEBUG", "1")
        .env("ASPECT_CREDENTIALS_FILE", root.join("credentials.json"))
        .env("DO_NOT_TRACK", "1")
        .output()
        .expect("spawn aspect-cli");
    let stderr = String::from_utf8_lossy(&out.stderr).into_owned();
    assert!(out.status.success(), "probe failed:\n{stderr}");

    let bodies = std::fs::read_to_string(&log)
        .unwrap_or_default()
        .split("--- end annotate ---\n")
        .filter(|b| !b.is_empty())
        .map(str::to_owned)
        .collect();
    (stderr, bodies)
}

#[test]
fn an_update_without_a_kind_is_skipped_quietly() {
    let (stderr, bodies) = run_probe();
    assert!(
        stderr.contains("Buildkite annotations: buildkite-agent probe ok"),
        "feature traces are off, so their absence proves nothing:\n{stderr}"
    );
    assert!(!stderr.contains("no renderer for kind"), "{stderr}");
    assert!(!stderr.contains("no context_id"), "{stderr}");
    assert!(!bodies.is_empty(), "no annotation was written:\n{stderr}");
}

#[test]
fn an_update_with_the_surface_kind_renders_its_progress() {
    let (stderr, bodies) = run_probe();
    assert!(
        bodies.iter().any(|b| b.contains("Running //probe...")),
        "no annotation carried the run-phase progress:\n{bodies:#?}\n{stderr}"
    );
}
