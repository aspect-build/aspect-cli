//! The Buildkite annotation surface, driven end to end: a fixture task emits
//! `phases.update`s under `BUILDKITE=true` with a fake `buildkite-agent` on
//! `PATH` that records every `annotate` body and `annotation remove` it
//! receives.
//!
//! Covered here because each case depends on the feature's per-task state:
//! an update with no `kind` is skipped without a trace, one carrying the
//! surface's kind re-renders with its progress text, a `SURFACE_REMOVE`
//! verdict latches so nothing re-posts, and a task declaring only
//! `phases.TRAITS` still renders.

#![cfg(unix)]

mod common;

use std::os::unix::fs::PermissionsExt;
use std::process::Command;

use common::{aspect_cli, base_env};

/// Emits an untemplated update, a run-phase update carrying the surface's
/// kind, then the terminal one. `{traits}` is the task's trait list.
const FIXTURE: &str = r#"
load("@aspect//private/lib/artifacts.axl", "artifacts")
load("@aspect//private/lib/bazel_results.axl", "init_data")
load("@aspect//private/lib/lifecycle.axl", "phases")
load("@aspect//private/lib/tips.axl", "tips")

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

probe = task(summary = "Test fixture.", implementation = _impl, traits = {traits})
"#;

const ALL_TRAITS: &str = "phases.TRAITS + artifacts.TRAITS + tips.TRAITS";

/// Retracts the annotation on the run-phase update, before the terminal one.
const REMOVE_ON_RUN: &str = r#"
load("@aspect//traits.axl", "Phases", "SURFACE_PUBLISH", "SURFACE_REMOVE")

def _retract_on_run(ctx, info):
    if "Running //probe..." in info.body:
        return SURFACE_REMOVE
    return SURFACE_PUBLISH

def config(ctx):
    ctx.traits[Phases].status_surface_update.append(_retract_on_run)
"#;

/// Logs `=== annotate` plus the body, or `=== remove`, to `$AGENT_LOG`.
const FAKE_AGENT: &str = r#"#!/bin/sh
case "$1 $2" in
  "annotate "*) { echo "=== annotate"; cat; } >> "$AGENT_LOG" ;;
  "annotation remove") echo "=== remove" >> "$AGENT_LOG" ;;
esac
exit 0
"#;

/// One `buildkite-agent` call the fake recorded.
#[derive(Debug)]
enum AgentCall {
    Annotate(String),
    Remove,
}

/// Runs `probe` from the fixture with `traits` and an optional
/// `.aspect/config.axl`, with trace logging on. Returns (stderr, agent calls).
fn run_probe(traits: &str, config: Option<&str>) -> (String, Vec<AgentCall>) {
    let dir = tempfile::tempdir().expect("temp dir");
    let root = dir.path();
    std::fs::write(root.join("MODULE.bazel"), "").expect("MODULE.bazel");
    std::fs::create_dir(root.join(".aspect")).expect(".aspect");
    std::fs::write(
        root.join(".aspect/fixture.axl"),
        FIXTURE.replace("{traits}", traits),
    )
    .expect("fixture");
    if let Some(config) = config {
        std::fs::write(root.join(".aspect/config.axl"), config).expect("config");
    }

    let bin = root.join("bin");
    std::fs::create_dir(&bin).expect("bin");
    let agent = bin.join("buildkite-agent");
    std::fs::write(&agent, FAKE_AGENT).expect("fake agent");
    std::fs::set_permissions(&agent, std::fs::Permissions::from_mode(0o755)).expect("chmod");
    let log = root.join("agent.log");

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

    let calls = std::fs::read_to_string(&log)
        .unwrap_or_default()
        .split("=== ")
        .filter(|c| !c.is_empty())
        .map(|c| match c.split_once('\n') {
            Some(("annotate", body)) => AgentCall::Annotate(body.to_owned()),
            Some(("remove", _)) => AgentCall::Remove,
            _ => panic!("unexpected agent log entry: {c:?}"),
        })
        .collect();
    (stderr, calls)
}

fn annotated(calls: &[AgentCall], needle: &str) -> bool {
    calls
        .iter()
        .any(|c| matches!(c, AgentCall::Annotate(body) if body.contains(needle)))
}

#[test]
fn an_update_without_a_kind_is_skipped_quietly() {
    let (stderr, calls) = run_probe(ALL_TRAITS, None);
    assert!(
        stderr.contains("Buildkite annotations: buildkite-agent probe ok"),
        "feature traces are off, so their absence proves nothing:\n{stderr}"
    );
    assert!(!stderr.contains("no renderer for kind"), "{stderr}");
    assert!(!stderr.contains("no context_id"), "{stderr}");
    assert!(!calls.is_empty(), "no annotation was written:\n{stderr}");
}

#[test]
fn an_update_with_the_surface_kind_renders_its_progress() {
    let (stderr, calls) = run_probe(ALL_TRAITS, None);
    assert!(
        annotated(&calls, "Running //probe..."),
        "no annotation carried the run-phase progress:\n{calls:#?}\n{stderr}"
    );
}

#[test]
fn a_removed_annotation_is_never_posted_again() {
    let (stderr, calls) = run_probe(ALL_TRAITS, Some(REMOVE_ON_RUN));
    let removed_at = calls
        .iter()
        .position(|c| matches!(c, AgentCall::Remove))
        .unwrap_or_else(|| {
            panic!("the hook's remove never reached the agent:\n{calls:#?}\n{stderr}")
        });
    assert!(
        calls[removed_at + 1..].is_empty(),
        "the annotation was posted again after its removal:\n{calls:#?}"
    );
}

#[test]
fn a_task_declaring_only_the_phases_traits_renders() {
    let (stderr, calls) = run_probe("phases.TRAITS", None);
    assert!(!stderr.contains("not found in TraitMap"), "{stderr}");
    assert!(
        annotated(&calls, "Running //probe..."),
        "no annotation was written:\n{calls:#?}\n{stderr}"
    );
}
