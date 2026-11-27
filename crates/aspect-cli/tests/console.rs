//! The `Console` feature's presets and templates change only the lines Aspect
//! prints around a task: task output, diagnostics, exit codes and the lifecycle
//! events CI surfaces consume are the same under every layout.
//!
//! The exceptions are `detail()` lines, which the preset (or `verbose`) shows
//! or hides, and `debug()` lines, which only the `debug` verbose level shows;
//! `info()` lines print under every layout.
//!
//! Spawns the real CLI so the feature's args, `config.axl` overrides, the
//! runtime-rendered header and verdict, and the AXL-rendered phase and progress
//! lines are exercised together. That includes where `file_hyperlink_format`
//! comes from: the command line, `ASPECT_FILE_HYPERLINK_FORMAT`, the `editor`
//! in the user's `~/.aspect/config.json` (a temp `$HOME`), the one in the
//! repository's `.aspect/config.json`, or `config.axl`.

mod common;

use axl_runtime::console::PRESETS;
use common::{aspect_cli, base_env};
use std::process::{Command, Output};

const TASKS: &str = r#"
load("@aspect//private/lib/environment.axl", "buildkite_section", "debug", "detail", "info", "warn")
load("@aspect//private/lib/lifecycle.axl", "Phase", "phases")

def _impl(ctx):
    ctx.std.io.stdout.write("preset=%s\n" % ctx.console.template_preset)
    warn(ctx.std, "task warning")
    detail(ctx.std, "task detail")
    debug(ctx.std, "task debug")
    info(ctx.std, "task info")
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
    args = {"code": args.int(default = 0), "flag": args.boolean(default = False)},
)

# The same kind in a group, so only the command path tells the two apart.
grouped_check = task(
    implementation = _impl,
    summary = "Exercise the console surface in a group.",
    group = ["fx"],
    kind = "check",
    args = {"code": args.int(default = 0), "flag": args.boolean(default = False)},
)

strict_check = check.alias(defaults = {"flag": True}, summary = "Check, warning on a flag.")

def _section(ctx):
    buildkite_section(ctx, ":rocket:", "Runner metadata")
    return 0

section = task(implementation = _section, summary = "Open a task-owned Buildkite section.")

def _conclude(ctx):
    return TaskConclusion(exit_code = 0, message = "nothing to do")

conclude = task(implementation = _conclude, summary = "End with an exit message.")
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
/// `config(ctx)`), under an empty `$HOME` so no user-level `config.axl`
/// applies.
fn run(config_body: &str, args: &[&str], buildkite: bool) -> Output {
    run_with_env(config_body, args, buildkite, &[])
}

/// [`run`] with `env` added to the CLI's environment.
fn run_with_env(config_body: &str, args: &[&str], buildkite: bool, env: &[(&str, &str)]) -> Output {
    run_with_files(config_body, args, buildkite, env, &[])
}

/// [`run_with_env`] with `files` written first, each at its path relative to
/// the workspace (`home/` is `$HOME`).
fn run_with_files(
    config_body: &str,
    args: &[&str],
    buildkite: bool,
    env: &[(&str, &str)],
    files: &[(&str, &str)],
) -> Output {
    let dir = tempfile::tempdir().expect("temp dir");
    std::fs::write(dir.path().join("MODULE.aspect"), "").expect("MODULE.aspect");
    let aspect = dir.path().join(".aspect");
    std::fs::create_dir(&aspect).expect(".aspect");
    for (path, content) in files {
        let path = dir.path().join(path);
        std::fs::create_dir_all(path.parent().expect("a parent")).expect("parent dir");
        std::fs::write(&path, content).expect("file");
    }
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
        .env("ASPECT_CLI_CACHE", dir.path().join("cache"))
        .env("HOME", dir.path().join("home"));
    if buildkite {
        cmd.env("BUILDKITE", "true");
    }
    cmd.envs(env.iter().copied());
    let output = cmd
        .output()
        .unwrap_or_else(|e| panic!("running `aspect {}`: {e}", args.join(" ")));
    dir.close().expect("removing temp dir");
    output
}

fn stderr(output: &Output) -> String {
    String::from_utf8_lossy(&output.stderr).into_owned()
}

/// The task's own output, its `info` line and the lifecycle events, which no
/// layout touches, and its `detail` line, which only the layouts that show
/// detail lines print.
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
    assert!(err.contains(": task info"), "{preset}: {err}");
    assert_eq!(
        err.contains(": task detail"),
        matches!(preset, "full" | "emoji"),
        "{preset}: {err}"
    );
}

/// `text` without its SGR sequences.
fn without_colour(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut rest = text;
    while let Some(at) = rest.find("\x1b[") {
        out.push_str(&rest[..at]);
        let after = &rest[at + 2..];
        let end = after.find('m').map_or(after.len(), |m| m + 1);
        rest = &after[end..];
    }
    out.push_str(rest);
    out
}

/// `full`, the default, gives each phase a heading and its updates `▹` lines
/// under it, with no emoji. A plain log and GitHub Actions, which only adds
/// colour (and a label from the CI run), read the same.
#[test]
fn full_lays_out_headings_and_updates_alike_in_a_plain_log_and_on_github_actions() {
    for env in [&[][..], &[("GITHUB_ACTIONS", "true")][..]] {
        let output = run_with_env("", &["check", "--code", "3"], false, env);
        assert_task_behaviour(&output, "full", 3);
        let err = without_colour(&stderr(&output));
        assert!(
            err.lines().any(|l| l.starts_with("▸ Running check")),
            "{env:?}: {err}"
        );
        for block in [
            "\n\n▸ Work\n▹ phase progress\n▹ heartbeat progress\n",
            "\n\n▸ Finish\n▹ final progress\n",
            "\n\n✗ Failed check",
            " (exit 3) in ",
        ] {
            assert!(err.contains(block), "{env:?}: {block:?} in\n{err}");
        }
        assert!(err.contains(" · task conclusion"), "{env:?}: {err}");
        for gone in ["→", "🎬", "❌", "✅", ":aspect:"] {
            assert!(!err.contains(gone), "{env:?}: {gone} in\n{err}");
        }
    }
}

/// On Buildkite `full` opens a section per phase, titled by its heading and
/// first update with no glyph; later updates are `▹` lines inside it, and the
/// verdict opens the last section expanded.
#[test]
fn full_on_buildkite_opens_a_section_per_phase() {
    let output = run("", &["check"], true);
    assert_task_behaviour(&output, "full", 0);
    let err = without_colour(&stderr(&output));
    let sections: Vec<&str> = err
        .lines()
        .filter(|l| l.starts_with("--- ") || l.starts_with("+++ ") || l.starts_with("^^^"))
        .collect();
    assert_eq!(sections.len(), 4, "{err}");
    assert!(
        sections[0].starts_with("--- :aspect: Running check"),
        "{err}"
    );
    assert_eq!(sections[1], "--- Work · phase progress", "{err}");
    assert_eq!(sections[2], "--- Finish · final progress", "{err}");
    assert!(sections[3].starts_with("+++ ✓ Passed check"), "{err}");
    assert!(err.lines().any(|l| l == "▹ heartbeat progress"), "{err}");
    let task = &err[err.find(sections[0]).expect("the task's section")..];
    assert!(!task.contains("\n\n"), "no blank lines: {task:?}");
}

/// `emoji` keeps the lines `full` printed before.
#[test]
fn emoji_keeps_the_previous_layout() {
    let output = run(
        "",
        &["check", "--console:template-preset=emoji", "--code", "3"],
        false,
    );
    assert_task_behaviour(&output, "emoji", 3);
    let err = stderr(&output);
    assert!(err.starts_with("→ 🎬 Running check task\n"), "{err}");
    for line in [
        "\n\n→ Work · phase progress\n→ heartbeat progress\n",
        "\n\n→ Finish · final progress\n",
        "\n\n→ ❌ Failed check task (exit 3) in ",
    ] {
        assert!(err.contains(line), "{line:?} in\n{err}");
    }
}

/// `plain` is gone, on the command line and in `config.axl` alike, and the
/// error says what took its place.
#[test]
fn plain_is_refused_naming_full_and_emoji() {
    const REMOVED_PLAIN: &str = r#"template preset "plain" was removed: "full" is the layout without emojis, and "emoji" the one with them"#;
    for (config, args) in [
        ("", &["check", "--console:template-preset=plain"][..]),
        (
            r#"ctx.features[Console].args.template_preset = "plain""#,
            &["check"][..],
        ),
        (
            r#"ctx.features[Console].args.task_presets = {"check": "plain"}"#,
            &["check"][..],
        ),
    ] {
        let output = run(config, args, false);
        let err = stderr(&output);
        assert_ne!(output.status.code(), Some(0), "{config} {args:?}: {err}");
        assert!(err.contains(REMOVED_PLAIN), "{config} {args:?}: {err}");
        if !config.is_empty() {
            assert!(
                !err.contains("--console:template-preset"),
                "a config.axl setting names no flag: {err}"
            );
        }
        assert!(
            output.stdout.is_empty(),
            "the task must not have run: {err}"
        );
    }
}

/// The runtime checks the preset's name, so the flag lists no values of its
/// own; its help names each preset, in the runtime's order, and no other.
#[test]
fn the_feature_offers_exactly_the_runtimes_presets() {
    let output = Command::new(aspect_cli())
        .args(["feature", "console"])
        .env_clear()
        .envs(base_env())
        .output()
        .expect("running `aspect feature console`");
    let help = String::from_utf8_lossy(&output.stdout);
    let mut lines = help
        .lines()
        .skip_while(|l| !l.contains("--console:template-preset"));
    let flag: String = lines
        .next()
        .into_iter()
        .chain(lines.take_while(|l| !l.contains("--console:")))
        .collect::<Vec<_>>()
        .join(" ");
    let at = |name: &str| flag.find(&format!("'{name}'"));
    let mut last = 0;
    for preset in PRESETS.map(|p| p.name) {
        let here = at(preset).unwrap_or_else(|| panic!("{preset} not in:\n{flag}"));
        assert!(here >= last, "{preset} out of order in:\n{flag}");
        last = here;
    }
    assert_eq!(at("plain"), None, "{flag}");
    assert!(!flag.contains("[possible values"), "{flag}");
}

/// On Buildkite every preset opens the verdict expanded, so it stays open
/// after a phase that ran Bazel opened its own group expanded.
#[test]
fn every_preset_opens_the_verdict_expanded_on_buildkite() {
    for preset in ["full", "emoji", "compact", "agent", "silent"] {
        let flag = format!("--console:template-preset={preset}");
        let output = run("", &["check", &flag], true);
        assert_task_behaviour(&output, preset, 0);
        let err = stderr(&output);
        let last = err
            .lines()
            .filter(|l| l.starts_with("--- ") || l.starts_with("+++ "))
            .last()
            .unwrap_or_else(|| panic!("{preset}: no sections in\n{err}"));
        assert!(last.starts_with("+++ "), "{preset}: {last:?}");
    }
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
fn a_recap_max_under_one_is_refused() {
    for value in ["0", "-3"] {
        let flag = format!("--console:recap-max={value}");
        let output = run("", &["check", &flag], false);
        let err = stderr(&output);
        assert_eq!(output.status.code(), Some(1), "{value}: {err}");
        assert!(
            err.contains(&format!(
                "{flag}: the most failures the summary lists must be 1 or more"
            )),
            "{value}: {err}"
        );
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
        verdict[0].starts_with("→ ❌ Failed check task (exit 3) in "),
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
    assert_eq!(
        err.trim(),
        "WARNING: task warning\nINFO: task info",
        "{err}"
    );
}

#[test]
fn buildkite_sections_print_under_every_preset() {
    for (preset, verdict) in [
        ("compact", "+++ :x: "),
        ("silent", "+++ :x: "),
        ("agent", "+++ \u{1b}[1;38;5;203mFailed"),
    ] {
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
        assert!(sections[4].starts_with(verdict), "{preset}: {err}");
        assert!(!err.contains("heartbeat progress"), "{preset}: {err}");
        if preset == "agent" {
            assert!(!err.contains(":aspect:"), "{err}");
        }
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
            "INFO: task info",
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
        stderr(&output).contains("▹ heartbeat progress"),
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
        err.contains(r#"task_templates["check"]: "verdcit" is not a slot; did you mean "verdict"? Expected one of banner, header, phase, progress, verdict, section, tip"#),
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
    assert!(err.contains("✓ Passed check task"), "{err}");
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

/// A preset with a blank header still prints the warning, first thing; on
/// Buildkite it follows the task's section marker, which every preset keeps.
#[test]
fn a_near_miss_warning_prints_under_a_blank_header() {
    for buildkite in [false, true] {
        let output = run(
            r#"ctx.features[Console].args.task_presets = {"chek": "silent"}"#,
            &["check", "--console:template-preset=silent"],
            buildkite,
        );
        assert_task_behaviour(&output, "silent", 0);
        let err = stderr(&output);
        let lines: Vec<&str> = err.lines().collect();
        let at = lines
            .iter()
            .position(|l| {
                l.ends_with(
                    r#"Console task_presets["chek"] names no task here; did you mean "check"?"#,
                )
            })
            .unwrap_or_else(|| panic!("no warning in:\n{err}"));
        if buildkite {
            assert!(
                at == 1 && lines[0].starts_with("--- ") && lines[0].contains("Running"),
                "{err}"
            );
        } else {
            assert_eq!(at, 0, "{err}");
        }
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
    assert!(err.contains("▸ Work\n▹ phase progress"), "{err}");
    assert!(err.contains("▸ Finish\n▹ final progress"), "{err}");
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
    assert!(err.contains("▸ Running check task"), "{err}");
    assert!(err.contains("▹ heartbeat progress"), "{err}");
}

/// The Workflows runner sections open through `buildkite_section`, so a
/// preset without emojis reaches them too, and off Buildkite they print
/// nothing.
#[test]
fn a_task_owned_buildkite_section_follows_the_preset() {
    for (preset, title) in [
        ("emoji", "--- :rocket: Runner metadata"),
        ("full", "--- Runner metadata"),
    ] {
        let flag = format!("--console:template-preset={preset}");
        let output = run("", &["section", &flag], true);
        let err = stderr(&output);
        assert_eq!(output.status.code(), Some(0), "{err}");
        assert!(err.lines().any(|l| l == title), "{preset}: {err}");
    }

    let output = run("", &["section"], false);
    let err = stderr(&output);
    assert_eq!(output.status.code(), Some(0), "{err}");
    assert!(!err.contains("Runner metadata"), "off Buildkite: {err}");
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
        "WARNING: task warning\nINFO: task info",
        "{}",
        stderr(&top)
    );

    let grouped = run(config, &["fx", "check"], false);
    assert_task_behaviour(&grouped, "compact", 0);
    let err = stderr(&grouped);
    assert!(err.contains("→ ✅ Passed fx check task in "), "{err}");
    assert!(!err.contains("Running"), "{err}");
}

/// Under a coding agent the default layout is `agent`; a preset chosen on the
/// command line or in `config.axl`, even `full`, keeps detection out.
#[test]
fn a_coding_agent_gets_the_agent_layout_unless_a_preset_is_chosen() {
    let agent = [("CLAUDECODE", "1")];
    let output = run_with_env("", &["check", "--code", "3"], false, &agent);
    assert_task_behaviour(&output, "agent", 3);
    let err = stderr(&output);
    for gone in ["Running", "phase progress", "❌", "🎬"] {
        assert!(!err.contains(gone), "{gone}: {err}");
    }
    let verdict: Vec<&str> = err.lines().filter(|l| l.contains("Failed")).collect();
    assert_eq!(verdict.len(), 1, "{err}");
    assert!(
        verdict[0].starts_with("→ Failed check task (exit 3) in "),
        "{err}"
    );

    let configured = run_with_env(
        r#"ctx.features[Console].args.template_preset = "full""#,
        &["check"],
        false,
        &agent,
    );
    assert_task_behaviour(&configured, "full", 0);

    let typed = run_with_env(
        "",
        &["check", "--console:template-preset=emoji"],
        false,
        &agent,
    );
    assert_task_behaviour(&typed, "emoji", 0);

    let per_task = run_with_env(
        r#"ctx.features[Console].args.task_presets = {"check": "compact"}"#,
        &["check"],
        false,
        &agent,
    );
    assert_task_behaviour(&per_task, "compact", 0);
}

/// A user-level `~/.aspect/config.axl` that sets `template_preset` keeps
/// detection out as a repository's does.
#[test]
fn a_user_level_preset_beats_coding_agent_detection() {
    let home = tempfile::tempdir().expect("temp home");
    let aspect = home.path().join(".aspect");
    std::fs::create_dir(&aspect).expect("~/.aspect");
    std::fs::write(
        aspect.join("config.axl"),
        "load(\"@aspect//feature/console.axl\", \"Console\")\n\ndef config(ctx):\n    ctx.features[Console].args.template_preset = \"full\"\n",
    )
    .expect("~/.aspect/config.axl");
    let home = home.path().to_str().expect("utf-8 home");
    let output = run_with_env(
        "",
        &["check"],
        false,
        &[("CLAUDECODE", "1"), ("HOME", home)],
    );
    assert_task_behaviour(&output, "full", 0);
}

#[test]
fn verbose_overrides_the_preset() {
    let shown = |config: &str, args: &[&str]| {
        let output = run(config, args, false);
        assert_eq!(output.status.code(), Some(0), "{}", stderr(&output));
        stderr(&output).contains("INFO: task detail")
    };
    assert!(shown(
        "",
        &[
            "check",
            "--console:template-preset=silent",
            "--console:verbose"
        ]
    ));
    assert!(!shown("", &["check", "--console:verbose=false"]));
    let hidden = "ctx.features[Console].args.verbose = False";
    assert!(!shown(hidden, &["check"]));
    assert!(shown(hidden, &["check", "--console:verbose=true"]));
}

/// A `debug()` line prints only at the `debug` level, however that is chosen,
/// and once: `ASPECT_DEBUG=1` adds no raw log record beside it.
#[test]
fn debug_lines_print_only_at_the_debug_level() {
    let count = |config: &str, args: &[&str], env: &[(&str, &str)]| {
        let output = run_with_env(config, args, false, env);
        assert_eq!(output.status.code(), Some(0), "{}", stderr(&output));
        stderr(&output).matches("DEBUG: task debug").count()
    };
    assert_eq!(count("", &["check"], &[]), 0);
    assert_eq!(count("", &["check", "--console:verbose"], &[]), 0);
    assert_eq!(count("", &["check", "--console:verbose=debug"], &[]), 1);
    assert_eq!(count("", &["check"], &[("ASPECT_VERBOSE", "debug")]), 1);
    assert_eq!(count("", &["check"], &[("ASPECT_DEBUG", "1")]), 1);
    let config = r#"ctx.features[Console].args.verbose = "debug""#;
    assert_eq!(count(config, &["check"], &[]), 1);
    assert_eq!(
        count(config, &["check"], &[("ASPECT_VERBOSE", "false")]),
        0,
        "the environment beats config.axl"
    );
    assert_eq!(
        count(
            "",
            &["check", "--console:verbose=false"],
            &[("ASPECT_DEBUG", "1")]
        ),
        0,
        "the command line beats the environment"
    );
}

/// The runtime's own debug records (`trace!`) print as `DEBUG:` lines at the
/// `debug` level, like AXL's, once each.
#[test]
fn runtime_debug_records_print_as_debug_lines() {
    let count = |env: &[(&str, &str)]| {
        let output = run_with_env("", &["check"], false, env);
        assert_eq!(output.status.code(), Some(0), "{}", stderr(&output));
        stderr(&output).matches("module @aspect at ").count()
    };
    assert_eq!(count(&[]), 0);
    assert_eq!(count(&[("ASPECT_VERBOSE", "debug")]), 1);
    assert_eq!(count(&[("ASPECT_DEBUG", "1")]), 1);
}

/// A `debug()` record reaches a log exporter whatever the level:
/// `OTEL_LOGS_EXPORTER=console` prints it raw without a `DEBUG:` line.
#[test]
fn a_console_log_exporter_prints_debug_records_at_any_level() {
    let output = run_with_env("", &["check"], false, &[("OTEL_LOGS_EXPORTER", "console")]);
    let err = stderr(&output);
    assert_eq!(output.status.code(), Some(0), "{err}");
    assert!(err.lines().any(|l| l == "task debug"), "{err}");
    assert!(!err.contains("DEBUG:"), "{err}");
}

/// Asking for console log records at the `debug` level prints each `debug()`
/// line twice, on purpose: once as a `DEBUG:` line and once as the raw record.
#[test]
fn a_console_log_exporter_at_the_debug_level_prints_both() {
    let output = run_with_env(
        "",
        &["check"],
        false,
        &[("ASPECT_DEBUG", "1"), ("OTEL_LOGS_EXPORTER", "console")],
    );
    let err = stderr(&output);
    assert_eq!(output.status.code(), Some(0), "{err}");
    assert_eq!(err.matches("DEBUG: task debug").count(), 1, "{err}");
    assert_eq!(
        err.lines().filter(|l| *l == "task debug").count(),
        1,
        "{err}"
    );
}

#[test]
fn an_invalid_aspect_verbose_names_the_closest_value() {
    let output = run_with_env("", &["check"], false, &[("ASPECT_VERBOSE", "debgu")]);
    assert_ne!(output.status.code(), Some(0));
    let err = stderr(&output);
    assert!(
        err.contains(r#"got "debgu"; did you mean "debug"?"#),
        "{err}"
    );
}

/// A task's exit message is its answer, so no preset hides it.
#[test]
fn an_exit_message_prints_under_every_preset() {
    for preset in ["full", "compact", "agent", "silent"] {
        let flag = format!("--console:template-preset={preset}");
        let output = run("", &["conclude", &flag], false);
        let err = stderr(&output);
        assert_eq!(output.status.code(), Some(0), "{preset}: {err}");
        assert!(err.contains("INFO: nothing to do"), "{preset}: {err}");
    }
}

/// Sets an invalid `file_hyperlink_format` in `config.axl`, which fails the
/// task only when nothing above it in precedence chose a format.
const BOGUS_CONFIG_FORMAT: &str = r#"ctx.features[Console].args.file_hyperlink_format = "bogus""#;

const USER_CONFIG_JSON: &str = "home/.aspect/config.json";

/// Whether the run failed on `config.axl`'s `file_hyperlink_format`, and on
/// nothing else.
fn config_format_refused(output: &Output) -> bool {
    let err = stderr(output);
    let refused = err.contains("Console file_hyperlink_format: ");
    assert_eq!(output.status.code() != Some(0), refused, "{err}");
    refused
}

#[test]
fn the_user_editor_beats_config_axl_and_loses_to_the_command_line_and_env() {
    let zed = [(USER_CONFIG_JSON, r#"{"editor": "zed"}"#)];
    let output = run_with_files(BOGUS_CONFIG_FORMAT, &["check"], false, &[], &[]);
    assert!(config_format_refused(&output), "no user config.json");
    let no_key = [(USER_CONFIG_JSON, r#"{"deployments": []}"#)];
    let output = run_with_files(BOGUS_CONFIG_FORMAT, &["check"], false, &[], &no_key);
    assert!(config_format_refused(&output), "no editor key");

    let output = run_with_files(BOGUS_CONFIG_FORMAT, &["check"], false, &[], &zed);
    assert!(!config_format_refused(&output), "editor > config.axl");

    let flag = ["check", "--console:file-hyperlink-format=bogus"];
    let output = run_with_files("", &flag, false, &[], &zed);
    let err = stderr(&output);
    assert_ne!(
        output.status.code(),
        Some(0),
        "command line > editor: {err}"
    );
    assert!(err.contains("\"bogus\" is not a hyperlink format"), "{err}");

    let env = [("ASPECT_FILE_HYPERLINK_FORMAT", "bogus")];
    let output = run_with_files("", &["check"], false, &env, &zed);
    let err = stderr(&output);
    assert_ne!(output.status.code(), Some(0), "env > editor: {err}");
    assert!(err.contains("ASPECT_FILE_HYPERLINK_FORMAT: "), "{err}");
}

#[test]
fn an_invalid_user_editor_warns_and_falls_back_to_config_axl() {
    let nope = [(USER_CONFIG_JSON, r#"{"editor": "nope"}"#)];
    let config = r#"ctx.features[Console].args.file_hyperlink_format = "zed""#;
    let output = run_with_files(config, &["check"], false, &[], &nope);
    let err = stderr(&output);
    assert_eq!(output.status.code(), Some(0), "{err}");
    let warning = err
        .lines()
        .find(|l| l.contains("WARNING") && l.contains("\"editor\""))
        .unwrap_or_else(|| panic!("no editor warning in:\n{err}"));
    assert!(warning.contains(".aspect/config.json"), "{warning}");
    assert!(
        warning.contains("\"nope\" is not a hyperlink format"),
        "{warning}"
    );

    let output = run_with_files(BOGUS_CONFIG_FORMAT, &["check"], false, &[], &nope);
    assert!(config_format_refused(&output), "falls back to config.axl");
}

/// The repository's `editor` is a team default: it beats `config.axl`, and the
/// user's `editor`, the command line and the environment each beat it. An
/// invalid one, or a `file://` format in either, warns, naming the file, and
/// falls through to `config.axl`.
#[test]
fn a_repositorys_editor_is_a_default_under_the_users() {
    const REPO_CONFIG_JSON: &str = ".aspect/config.json";
    let zed = [(REPO_CONFIG_JSON, r#"{"editor": "zed"}"#)];
    let output = run_with_files(BOGUS_CONFIG_FORMAT, &["check"], false, &[], &zed);
    assert!(!config_format_refused(&output), "repo editor > config.axl");

    let flag = ["check", "--console:file-hyperlink-format=bogus"];
    let output = run_with_files("", &flag, false, &[], &zed);
    assert_ne!(output.status.code(), Some(0), "command line > repo editor");
    let env = [("ASPECT_FILE_HYPERLINK_FORMAT", "bogus")];
    let output = run_with_files("", &["check"], false, &env, &zed);
    assert_ne!(output.status.code(), Some(0), "env > repo editor");

    // The user's `editor` beats the repository's: an invalid repository one
    // under a valid user's is never read, so it neither warns nor fails.
    let both = [
        (REPO_CONFIG_JSON, r#"{"editor": "nope"}"#),
        (USER_CONFIG_JSON, r#"{"editor": "cursor"}"#),
    ];
    let output = run_with_files(BOGUS_CONFIG_FORMAT, &["check"], false, &[], &both);
    assert!(!config_format_refused(&output), "user editor > repo editor");
    assert!(
        !stderr(&output).contains("\"editor\""),
        "{}",
        stderr(&output)
    );

    let nope = [(REPO_CONFIG_JSON, r#"{"editor": "nope"}"#)];
    let output = run_with_files(BOGUS_CONFIG_FORMAT, &["check"], false, &[], &nope);
    let err = stderr(&output);
    let warning = err
        .lines()
        .find(|l| l.contains("WARNING") && l.contains("\"editor\""))
        .unwrap_or_else(|| panic!("no editor warning in:\n{err}"));
    assert!(!warning.contains("home/"), "names the repo's: {warning}");
    assert!(warning.contains(REPO_CONFIG_JSON), "{warning}");
    assert!(config_format_refused(&output), "falls back to config.axl");

    // A repository's data file names an editor, never a URL template, which
    // stays the user's.
    for refused in ["myeditor://{path}"] {
        let json = format!(r#"{{"editor": "{refused}"}}"#);
        let repo = [(REPO_CONFIG_JSON, json.as_str())];
        let output = run_with_files(BOGUS_CONFIG_FORMAT, &["check"], false, &[], &repo);
        let err = stderr(&output);
        assert!(
            err.contains("a repository's config.json takes auto, none or an editor's name"),
            "{refused}: {err}"
        );
        assert!(
            config_format_refused(&output),
            "{refused}: falls back to config.axl"
        );
        let user = [(USER_CONFIG_JSON, json.as_str())];
        let output = run_with_files(BOGUS_CONFIG_FORMAT, &["check"], false, &[], &user);
        assert!(
            !config_format_refused(&output),
            "the user's may be {refused}"
        );
    }

    // A file:// format is no one's: from either file it warns, naming the
    // file, and falls through to config.axl.
    for refused in ["file", "kitty"] {
        let json = format!(r#"{{"editor": "{refused}"}}"#);
        for (file, kind) in [(REPO_CONFIG_JSON, "repo"), (USER_CONFIG_JSON, "user")] {
            let files = [(file, json.as_str())];
            let output = run_with_files(BOGUS_CONFIG_FORMAT, &["check"], false, &[], &files);
            let err = stderr(&output);
            let warning = err
                .lines()
                .find(|l| l.contains("WARNING") && l.contains("\"editor\""))
                .unwrap_or_else(|| panic!("{kind} {refused}: no editor warning in:\n{err}"));
            assert!(warning.contains(file), "{warning}");
            assert!(warning.contains("a file:// link"), "{warning}");
            assert!(
                config_format_refused(&output),
                "{kind} {refused}: falls back to config.axl"
            );
        }
    }
}
