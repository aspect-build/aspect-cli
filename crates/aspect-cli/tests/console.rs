//! The `Console` feature's presets and templates change only the lines Aspect
//! prints around a task: task output, diagnostics, exit codes and the lifecycle
//! events CI surfaces consume are the same under every layout.
//!
//! Spawns the real CLI so the feature's args, `config.axl` overrides, the
//! runtime-rendered header and verdict, and the AXL-rendered phase and progress
//! lines are exercised together.

mod common;

use axl_runtime::console::PRESETS;
use common::{aspect_cli, base_env};
use std::process::{Command, Output};

const TASKS: &str = r#"
load("@aspect//private/lib/environment.axl", "buildkite_section", "warn")
load("@aspect//private/lib/lifecycle.axl", "Phase", "Phases", "phases")

def _impl(ctx):
    ctx.std.io.stdout.write("preset=%s\n" % ctx.console.template_preset)
    warn(ctx.std, "task warning")
    ph = phases.new(ctx, {})
    ph.update("running", "phase progress", phase = Phase(name = "work"))
    ph.update("running", "heartbeat progress")
    return ph.update(
        "warning" if ctx.args.flag else ("failed" if ctx.args.code else "passed"),
        "final progress",
        phase = Phase(name = "finish"),
        final = True,
        conclusion = "task conclusion",
        exit_code = ctx.args.code,
    )

check = task(
    implementation = _impl,
    summary = "Exercise the console surface.",
    traits = [Phases],
    args = {"code": args.int(default = 0), "flag": args.boolean(default = False)},
)

# The same kind in a group, so only the command path tells the two apart.
grouped_check = task(
    implementation = _impl,
    summary = "Exercise the console surface in a group.",
    group = ["fx"],
    kind = "check",
    traits = [Phases],
    args = {"code": args.int(default = 0), "flag": args.boolean(default = False)},
)

strict_check = check.alias(defaults = {"flag": True}, summary = "Check, warning on a flag.")

def _section(ctx):
    buildkite_section(ctx, ":rocket:", "Runner metadata")
    return 0

section = task(implementation = _section, summary = "Open a task-owned Buildkite section.")
"#;

/// Writes every lifecycle event to stdout, so a layout that hid one would show.
const OBSERVE: &str = r#"
load("@aspect//feature/buildkite_annotations.axl", "BuildkiteAnnotations")
load("@aspect//private/lib/lifecycle.axl", "Phases")

def _observe(ctx, update):
    ctx.std.io.stdout.write("event:%s:%s\n" % (update.status, update.final))

def config(ctx):
    ctx.features[BuildkiteAnnotations].args.enabled = False
    ctx.traits[Phases].task_update.append(_observe)
"#;

const EVENTS: &str = "event:running:False\nevent:running:False\n";

/// Run `aspect <args>` in a scratch workspace with `tasks.axl` and a
/// `config.axl` of `OBSERVE` followed by `config_body` (statements run inside
/// `config(ctx)`).
fn run(config_body: &str, args: &[&str], buildkite: bool) -> Output {
    let dir = tempfile::tempdir().expect("temp dir");
    std::fs::write(dir.path().join("MODULE.aspect"), "").expect("MODULE.aspect");
    let aspect = dir.path().join(".aspect");
    std::fs::create_dir(&aspect).expect(".aspect");
    std::fs::write(aspect.join("tasks.axl"), TASKS).expect("tasks.axl");
    let config = format!(
        "load(\"@aspect//feature/console.axl\", \"Console\")\n{OBSERVE}{}",
        config_body
            .lines()
            .map(|l| format!("    {l}\n"))
            .collect::<String>()
    );
    std::fs::write(aspect.join("config.axl"), config).expect("config.axl");
    let mut cmd = Command::new(aspect_cli());
    cmd.args(args)
        .current_dir(dir.path())
        .env_clear()
        .envs(base_env())
        .env("ASPECT_CLI_CACHE", dir.path().join("cache"));
    if buildkite {
        cmd.env("BUILDKITE", "true");
    }
    let output = cmd
        .output()
        .unwrap_or_else(|e| panic!("running `aspect {}`: {e}", args.join(" ")));
    dir.close().expect("removing temp dir");
    output
}

fn stderr(output: &Output) -> String {
    String::from_utf8_lossy(&output.stderr).into_owned()
}

/// The task's own output and the lifecycle events, which no layout touches.
fn assert_task_behaviour(output: &Output, preset: &str, code: i32) {
    let err = stderr(output);
    assert_eq!(output.status.code(), Some(code), "{err}");
    let status = if code == 0 { "passed" } else { "failed" };
    assert_eq!(
        String::from_utf8_lossy(&output.stdout),
        format!("preset={preset}\n{EVENTS}event:{status}:True\n"),
        "{err}"
    );
    assert!(err.contains("task warning"), "{err}");
}

#[test]
fn the_feature_offers_exactly_the_runtimes_presets() {
    let output = Command::new(aspect_cli())
        .args(["feature", "console"])
        .env_clear()
        .envs(base_env())
        .output()
        .expect("running `aspect feature console`");
    let help = String::from_utf8_lossy(&output.stdout);
    let values = help
        .lines()
        .skip_while(|l| !l.contains("--console:template-preset"))
        .find_map(|l| l.split_once("[possible values: "))
        .and_then(|(_, rest)| rest.split_once(']'))
        .map(|(values, _)| values.split(", ").collect::<Vec<_>>())
        .unwrap_or_else(|| panic!("no template-preset values in:\n{help}"));
    let presets: Vec<&str> = PRESETS.iter().map(|p| p.name).collect();
    assert_eq!(values, presets);
}

#[test]
fn every_preset_keeps_task_output_events_and_exit_codes() {
    for preset in PRESETS.map(|p| p.name) {
        for code in ["0", "3"] {
            let flag = format!("--console:template-preset={preset}");
            let output = run("", &["check", &flag, "--code", code], false);
            assert_task_behaviour(&output, preset, code.parse().unwrap());
        }
    }
}

#[test]
fn compact_prints_the_verdict_alone() {
    let output = run(
        "",
        &["check", "--console:template-preset=compact", "--code", "3"],
        false,
    );
    let err = stderr(&output);
    for gone in ["Running", "phase progress", "heartbeat progress", "Work"] {
        assert!(!err.contains(gone), "{gone}: {err}");
    }
    let verdict: Vec<&str> = err.lines().filter(|l| l.contains("Failed")).collect();
    assert_eq!(verdict.len(), 1, "{err}");
    assert!(
        verdict[0].starts_with("→ ❌ Failed check task (exit code 3) in "),
        "{err}"
    );
    assert!(verdict[0].ends_with(" · task conclusion"), "{err}");
}

#[test]
fn silent_prints_nothing_of_aspects_own() {
    let output = run(
        "",
        &["check", "--console:template-preset=silent", "--code", "3"],
        false,
    );
    let err = stderr(&output);
    assert_eq!(err.trim(), "WARNING: task warning", "{err}");
}

#[test]
fn buildkite_sections_print_under_every_preset() {
    for preset in ["compact", "silent"] {
        let flag = format!("--console:template-preset={preset}");
        let output = run("", &["check", &flag, "--code", "3"], true);
        assert_task_behaviour(&output, preset, 3);
        let err = stderr(&output);
        let sections: Vec<&str> = err
            .lines()
            .filter(|l| l.starts_with("--- ") || l.starts_with("+++ ") || *l == "^^^ +++")
            .collect();
        assert_eq!(sections.len(), 5, "{preset}: {err}");
        assert!(sections[0].contains("Running"), "{preset}: {err}");
        assert_eq!(sections[3], "^^^ +++", "{preset}: {err}");
        assert!(sections[4].starts_with("+++ :x: "), "{preset}: {err}");
        assert!(!err.contains("heartbeat progress"), "{preset}: {err}");
    }
}

#[test]
fn config_templates_override_the_preset_then_task_templates() {
    let output = run(
        r#"ctx.features[Console].args.template_preset = "silent"
ctx.features[Console].args.templates = {
    "header": "start {{ task.path }}",
    "progress": "· {{ text }}",
}
ctx.features[Console].args.task_templates = {
    "check": {"verdict": "{{ verdict }} {{ exit_code }} {{ phases | map(attribute='name') | join(',') }}"},
}"#,
        &["check", "--code", "3"],
        false,
    );
    assert_task_behaviour(&output, "silent", 3);
    let err = stderr(&output);
    let lines: Vec<&str> = err.lines().collect();
    assert_eq!(
        lines,
        [
            "start check",
            "WARNING: task warning",
            "· heartbeat progress",
            "failed 3 init,work,finish"
        ],
        "{err}"
    );
}

#[test]
fn a_cli_preset_beats_the_config_preset() {
    let output = run(
        r#"ctx.features[Console].args.template_preset = "silent""#,
        &["check", "--console:template-preset=compact"],
        false,
    );
    assert_task_behaviour(&output, "compact", 0);
    assert!(
        stderr(&output).contains("→ ✅ Passed check task"),
        "{}",
        stderr(&output)
    );
}

/// A preset typed for one run is what the user asked to see, so it beats a
/// task's `task_presets` entry too.
#[test]
fn a_cli_preset_beats_task_presets() {
    let output = run(
        r#"ctx.features[Console].args.task_presets = {"check": "silent"}"#,
        &["check", "--console:template-preset=full"],
        false,
    );
    assert_task_behaviour(&output, "full", 0);
    assert!(
        stderr(&output).contains("→ heartbeat progress"),
        "{}",
        stderr(&output)
    );
}

#[test]
fn a_template_that_cannot_compile_fails_before_the_task_runs() {
    let output = run(
        r#"ctx.features[Console].args.task_templates = {"check": {"verdict": "{% if %}"}}"#,
        &["check"],
        false,
    );
    let err = stderr(&output);
    assert_eq!(output.status.code(), Some(1), "{err}");
    assert!(
        err.contains(r#"task_templates["check"]["verdict"] does not compile"#),
        "{err}"
    );
    assert!(
        output.stdout.is_empty(),
        "the task must not have run: {err}"
    );
}

#[test]
fn a_misnamed_slot_is_refused() {
    let output = run(
        r#"ctx.features[Console].args.task_templates = {"check": {"verdcit": "x"}}"#,
        &["check"],
        false,
    );
    let err = stderr(&output);
    assert_eq!(output.status.code(), Some(1), "{err}");
    assert!(
        err.contains(r#"task_templates["check"]: "verdcit" is not a slot"#),
        "{err}"
    );
}

/// A user-level `config.axl` reaches every repo, so naming a task only some of
/// them define must not break the others.
#[test]
fn an_entry_for_a_task_this_repo_lacks_is_ignored() {
    let output = run(
        r#"ctx.features[Console].args.task_templates = {"deploy": {"verdict": "deployed"}}
ctx.features[Console].args.task_presets = {"deploy": "silent"}"#,
        &["check"],
        false,
    );
    assert_task_behaviour(&output, "full", 0);
    let err = stderr(&output);
    assert!(err.contains("→ ✅ Passed check task"), "{err}");
    assert!(!err.contains("deployed"), "{err}");
    assert!(!err.contains("names no task here"), "{err}");
}

/// A key a slip away from a task here is most likely a typo, so it is named
/// once, whichever args carry it, on `Console` and on a CI surface alike, and
/// under the header, so on Buildkite it lands in the task's section.
#[test]
fn a_near_miss_task_key_is_warned_about_once() {
    let output = run(
        r#"ctx.features[Console].args.task_templates = {"chek": {"verdict": "x"}, "fx chek": {"verdict": "y"}}
ctx.features[Console].args.task_presets = {"chek": "silent", "strict-chek": "silent"}
ctx.features[BuildkiteAnnotations].args.enabled = True
ctx.features[BuildkiteAnnotations].args.task_presets = {"fx chekc": "full"}"#,
        &["check"],
        false,
    );
    assert_task_behaviour(&output, "full", 0);
    let err = stderr(&output);
    let header = err
        .find("Running check")
        .unwrap_or_else(|| panic!("no header in:\n{err}"));
    for warning in [
        r#"Console task_presets["chek"] and task_templates["chek"] name no task here; did you mean "check"?"#,
        r#"Console task_templates["fx chek"] names no task here; did you mean "fx check"?"#,
        r#"Console task_presets["strict-chek"] names no task here; did you mean "strict-check"?"#,
        r#"BuildkiteAnnotations task_presets["fx chekc"] names no task here; did you mean "fx check"?"#,
    ] {
        assert_eq!(err.matches(warning).count(), 1, "{warning}\n{err}");
        assert!(err.find(warning) > Some(header), "{warning}\n{err}");
    }
}

#[test]
fn a_template_that_fails_to_render_falls_back_to_its_preset_with_one_warning() {
    let output = run(
        r#"ctx.features[Console].args.templates = {"phase": "{{ text.missing.deeper }}"}"#,
        &["check"],
        false,
    );
    assert_task_behaviour(&output, "full", 0);
    let err = stderr(&output);
    assert_eq!(
        err.matches("Console template for \"phase\" failed to render")
            .count(),
        1,
        "{err}"
    );
    assert!(err.contains("→ Work · phase progress"), "{err}");
    assert!(err.contains("→ Finish · final progress"), "{err}");
}

#[test]
fn a_flagged_task_renders_as_flagged() {
    let output = run(
        r#"ctx.features[Console].args.templates = {"verdict": "{{ verdict }} {{ glyph }} {{ verb }}"}"#,
        &["check", "--flag"],
        false,
    );
    let err = stderr(&output);
    assert_eq!(output.status.code(), Some(0), "{err}");
    assert_eq!(err.lines().last(), Some("flagged ⚠️  Flagged"), "{err}");
}

#[test]
fn a_phase_template_titles_its_buildkite_section() {
    let output = run(
        r#"ctx.features[Console].args.templates = {"phase": "P {{ title }}"}"#,
        &["check"],
        true,
    );
    assert_task_behaviour(&output, "full", 0);
    let err = stderr(&output);
    assert!(err.lines().any(|l| l == "--- P Work"), "{err}");
    assert!(err.lines().any(|l| l == "--- P Finish"), "{err}");
}

#[test]
fn disabling_the_feature_keeps_the_full_layout() {
    let output = run(
        r#"ctx.features[Console].enabled = False
ctx.features[Console].args.template_preset = "silent""#,
        &["check"],
        false,
    );
    let err = stderr(&output);
    assert!(err.contains("→ 🎬 Running check task"), "{err}");
    assert!(err.contains("→ heartbeat progress"), "{err}");
}

/// The Workflows runner sections open through `buildkite_section`, so `plain`
/// reaches them too.
#[test]
fn a_task_owned_buildkite_section_follows_the_preset() {
    for (preset, title) in [
        ("full", "--- :rocket: Runner metadata"),
        ("plain", "--- Runner metadata"),
    ] {
        let flag = format!("--console:template-preset={preset}");
        let output = run("", &["section", &flag], true);
        let err = stderr(&output);
        assert_eq!(output.status.code(), Some(0), "{err}");
        assert!(err.lines().any(|l| l == title), "{preset}: {err}");
    }
}

/// `task_presets` picks one task's preset, and a command-path entry tells a
/// grouped task from a top-level one of the same kind.
#[test]
fn task_presets_pick_a_preset_by_kind_or_command_path() {
    let config = r#"ctx.features[Console].args.templates = {"progress": "everyone"}
ctx.features[Console].args.task_presets = {"check": "silent", "fx check": "compact"}"#;
    let top = run(config, &["check"], false);
    assert_task_behaviour(&top, "silent", 0);
    assert_eq!(
        stderr(&top).trim(),
        "WARNING: task warning",
        "{}",
        stderr(&top)
    );

    let grouped = run(config, &["fx", "check"], false);
    assert_task_behaviour(&grouped, "compact", 0);
    let err = stderr(&grouped);
    assert!(err.contains("→ ✅ Passed fx check task in "), "{err}");
    assert!(!err.contains("Running"), "{err}");
}
