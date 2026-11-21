//! A mistyped name from a closed set fails, naming the near miss when there is
//! one: a task path in `ctx.tasks`, an arg, a feature name, a task in
//! `aspect describe`, a `status_badges` key. A far miss keeps the plain error.
//!
//! Spawns the real CLI so the error reaches the user as it would in a repo.

mod common;

use common::{aspect_cli, base_env};
use std::process::{Command, Output};

const TASKS: &str = r#"
def _impl(ctx):
    return 0

check = task(
    implementation = _impl,
    summary = "Do nothing.",
    args = {"mode": args.string(default = "x")},
)
"#;

/// Run `aspect <args>` in a scratch workspace with `tasks.axl` and a
/// `config.axl` whose `config(ctx)` runs `config_body`.
fn run(config_body: &str, args: &[&str]) -> Output {
    let dir = tempfile::tempdir().expect("temp dir");
    std::fs::write(dir.path().join("MODULE.aspect"), "").expect("MODULE.aspect");
    let aspect = dir.path().join(".aspect");
    std::fs::create_dir(&aspect).expect(".aspect");
    std::fs::write(aspect.join("tasks.axl"), TASKS).expect("tasks.axl");
    let config = format!(
        r#"load("@aspect//feature/console.axl", "Console")
load("@aspect//feature/github_status_comments.axl", "GithubStatusComments")
load("@aspect//feature/gitlab_status_comments.axl", "GitlabStatusComments")

def config(ctx):
    pass
{}"#,
        config_body
            .lines()
            .map(|l| format!("    {l}\n"))
            .collect::<String>()
    );
    std::fs::write(aspect.join("config.axl"), config).expect("config.axl");
    let output = Command::new(aspect_cli())
        .args(args)
        .current_dir(dir.path())
        .env_clear()
        .envs(base_env())
        .env("ASPECT_CLI_CACHE", dir.path().join("cache"))
        .output()
        .unwrap_or_else(|e| panic!("running `aspect {}`: {e}", args.join(" ")));
    dir.close().expect("removing temp dir");
    output
}

/// The run failed, and its stderr contains `want` and says `did you mean`
/// only if `want` does.
fn assert_fails_with(output: &Output, want: &str) {
    let err = String::from_utf8_lossy(&output.stderr);
    assert_ne!(output.status.code(), Some(0), "{err}");
    assert!(err.contains(want), "want {want:?} in:\n{err}");
    assert_eq!(
        err.contains("did you mean"),
        want.contains("did you mean"),
        "{err}"
    );
}

#[test]
fn a_task_path_near_miss_is_suggested() {
    assert_fails_with(
        &run(r#"ctx.tasks["chek"].args.mode = "y""#, &["check"]),
        r#"no task found at path "chek"; did you mean "check"?"#,
    );
    assert_fails_with(
        &run(r#"ctx.tasks["deploy"].args.mode = "y""#, &["check"]),
        r#"no task found at path "deploy""#,
    );
}

#[test]
fn an_arg_near_miss_is_suggested() {
    assert_fails_with(
        &run(r#"ctx.tasks["check"].args.mdoe = "y""#, &["check"]),
        r#"no such arg `mdoe`; valid args are: mode; did you mean "mode"?"#,
    );
    assert_fails_with(
        &run(
            r#"ctx.features[Console].args.templte_preset = "plain""#,
            &["check"],
        ),
        r#"did you mean "template_preset"?"#,
    );
    assert_fails_with(
        &run(r#"ctx.tasks["check"].args.verbose = True"#, &["check"]),
        "no such arg `verbose`; valid args are: mode",
    );
    assert_fails_with(
        &run(r#"print(ctx.tasks["check"].args.mdoe)"#, &["check"]),
        "has no attribute `mdoe`, did you mean `mode`?",
    );
}

#[test]
fn a_feature_near_miss_is_suggested() {
    let output = run("", &["feature", "consle"]);
    assert_eq!(output.status.code(), Some(2));
    assert_fails_with(&output, r#"; did you mean "console"?"#);
    assert_fails_with(
        &run("", &["feature", "nonesuch"]),
        r#"unknown feature "nonesuch". Valid features: "#,
    );
}

#[test]
fn a_described_task_near_miss_is_suggested() {
    assert_fails_with(
        &run("", &["describe", "chekc"]),
        r#"no task "chekc"; did you mean "check"? Run `aspect describe`"#,
    );
    assert_fails_with(
        &run("", &["describe", "nonesuch"]),
        r#"no task "nonesuch". Run `aspect describe`"#,
    );
}

#[test]
fn an_unknown_status_badge_fails_when_the_feature_loads() {
    for feature in ["GithubStatusComments", "GitlabStatusComments"] {
        assert_fails_with(
            &run(
                &format!(r#"ctx.features[{feature}].args.status_badges = {{"pased": "ok"}}"#),
                &["check"],
            ),
            &format!(
                r#"{feature} status_badges: "pased" is not a badge; expected one of aborted, failed, passed, running, warning; did you mean "passed"?"#
            ),
        );
    }
    assert_fails_with(
        &run(
            r#"ctx.features[GithubStatusComments].args.status_badges = {"skipped": "ok"}"#,
            &["check"],
        ),
        r#"GithubStatusComments status_badges: "skipped" is not a badge"#,
    );
    let output = run(
        r#"ctx.features[GithubStatusComments].args.status_badges = {"passed": "ok"}"#,
        &["check"],
    );
    assert_eq!(
        output.status.code(),
        Some(0),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
}
