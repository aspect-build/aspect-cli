//! A `.aspect/*.axl` file the user broke must not cost them the commands they
//! need to find it.
//!
//! The CLI's whole surface is defined in AXL, so a single unparsable file used
//! to fail every command — `aspect help`, the way out, included. The policy
//! these tests pin (stated in `docs/design.md`, "When a file does not load",
//! and carried out by `Unloadable` in `main.rs`): `version` and `help` run
//! anyway and report the failure as a warning, while anything that dispatches
//! a task still fails on it. Both halves matter. A silently swallowed syntax
//! error would be worse than the hard failure it replaced, so every degraded
//! run is checked for the diagnostic as well as the exit code — and a command
//! that *does* fail is checked for the files it did not raise, since dropping
//! those would make what the user hears about depend on `read_dir` order.
//!
//! Also pinned here: each warning names what that particular file's absence
//! costs, a `use_feature` file defining no tasks; one file named twice is
//! reported once; the diagnostic carries exactly one `error:` prefix, Starlark
//! rendering its own and the CLI having once added a second; and the
//! `MODULE.aspect` read before the tolerance begins names itself when it
//! cannot be read at all.
//!
//! Every run gets a throwaway `$HOME` so the developer's own
//! `~/.aspect/config.axl` cannot join the run and change what is reported.

mod common;

use common::aspect_cli;
#[cfg(unix)]
use std::os::unix::fs::PermissionsExt;
use std::path::Path;
use std::process::{Command, Output};

/// A task file that parses, so a degraded `help` has something of the user's
/// own left to list.
const GOOD_SCRIPT: &str = r#"
def _impl(ctx: TaskContext) -> int:
    return 0

goodtask = task(summary = "A task that loads.", implementation = _impl)
"#;

/// The reported shape: a typo that cannot parse. `is` is a reserved keyword,
/// so this fails in the parser rather than at runtime.
const UNPARSABLE_SCRIPT: &str = "\nthis is not valid starlark(\n";

/// A `config.axl` that parses and then fails when its `config()` runs, which
/// is the other way a hand-edited file breaks the startup phases.
const FAILING_CONFIG: &str = r#"
def config(ctx):
    ctx.tasks["no-such-task"].args.anything = 1
"#;

/// A `config.axl` whose author chose to end the run — a repo-level gate, the
/// shape of "this checkout needs a newer CLI than you have".
const EXITING_CONFIG: &str = r#"
def config(ctx):
    ctx.std.process.exit(3, "this repo requires a newer aspect CLI")
"#;

/// A script that defines `mytask` and *then* fails to parse, so a config
/// referring to `mytask` fails as a consequence rather than on its own.
const UNPARSABLE_SCRIPT_DEFINING_MYTASK: &str = r#"
def _impl(ctx: TaskContext) -> int:
    return 0

mytask = task(summary = "mine", implementation = _impl)

this is not valid starlark(
"#;

/// A `config.axl` that is correct in itself: it only configures `mytask`.
const CONFIG_NEEDING_MYTASK: &str = r#"
def config(ctx):
    ctx.tasks["mytask"].args.something = "true"
"#;

/// How to run one fixture. Built by [`Fixture::new`] and driven by
/// [`Fixture::run`]; the helpers below cover the common shapes.
struct Fixture<'a> {
    /// `(path, contents)` pairs, relative to the workspace root. Written in
    /// the order given, which is also the order `read_dir` reports them in on
    /// the filesystems we test on — see
    /// `a_file_failure_recorded_before_a_gate_is_still_reported`. `home/` is
    /// the run's throwaway `$HOME`, so a fixture that needs a user-global
    /// `~/.aspect/config.axl` writes `home/.aspect/config.axl`.
    files: &'a [(&'a str, &'a str)],
    /// Directory to run from, relative to the workspace root. The CLI searches
    /// `.aspect` from the aspect root down to the cwd, so a nested config is
    /// only picked up when the cwd is at or below it.
    cwd: &'a str,
    /// Whether to set `ASPECT_DEBUG`, which every other case clears.
    debug: bool,
    /// Paths, relative to the workspace root, to `chmod 000` once written.
    /// Failing the *read* of a file whose contents are fine is its own error
    /// path, separate from every parse and eval failure here. Unix only, and
    /// no obstacle to root, so a fixture using it needs an unprivileged run —
    /// which is what the hosted CI runners give us.
    unreadable: &'a [&'a str],
}

impl<'a> Fixture<'a> {
    fn new(files: &'a [(&'a str, &'a str)]) -> Self {
        Fixture {
            files,
            cwd: ".",
            debug: false,
            unreadable: &[],
        }
    }

    /// Build the workspace, run `aspect <args>` in it, and hand back the
    /// output together with the workspace, which the caller may inspect for
    /// what the run left behind before dropping it.
    fn run(&self, args: &[&str]) -> (Output, tempfile::TempDir) {
        let dir = tempfile::tempdir().expect("temp dir");
        let home = dir.path().join("home");
        std::fs::create_dir(&home).expect("home");
        std::fs::write(dir.path().join("MODULE.bazel"), "").expect("MODULE.bazel");
        std::fs::create_dir(dir.path().join(".aspect")).expect(".aspect");
        for (path, contents) in self.files {
            let path = dir.path().join(path);
            std::fs::create_dir_all(path.parent().expect("a fixture path has a parent"))
                .expect("fixture parent");
            std::fs::write(&path, contents).expect("writing a fixture");
        }
        #[cfg(unix)]
        for path in self.unreadable {
            let path = dir.path().join(path);
            let mut perms = std::fs::metadata(&path)
                .expect("a path to chmod")
                .permissions();
            perms.set_mode(0o000);
            std::fs::set_permissions(&path, perms).expect("chmod 000");
        }
        let cwd = dir.path().join(self.cwd);
        std::fs::create_dir_all(&cwd).expect("cwd");

        let mut cmd = Command::new(aspect_cli());
        let cmd = cmd.args(args).current_dir(&cwd).env("HOME", &home).env(
            "ASPECT_CREDENTIALS_FILE",
            dir.path().join("credentials.json"),
        );
        let output = if self.debug {
            cmd.env("ASPECT_DEBUG", "1")
        } else {
            cmd.env_remove("ASPECT_DEBUG")
        }
        .output()
        .unwrap_or_else(|e| panic!("running `aspect {}`: {e}", args.join(" ")));
        (output, dir)
    }
}

/// Run `aspect <args>` in a scratch workspace whose `.aspect/` holds `files`,
/// each a `(name, contents)` pair.
fn run_in_workspace(files: &[(&str, &str)], args: &[&str]) -> Output {
    let rooted = under_dot_aspect(files);
    let rooted: Vec<(&str, &str)> = rooted.iter().map(|(p, c)| (p.as_str(), *c)).collect();
    run_in_rooted_workspace(&rooted, args)
}

/// [`run_in_workspace`] with `ASPECT_DEBUG` set, which is the escape hatch
/// for the config diagnostic a derived failure suppresses.
fn run_in_debug_workspace(files: &[(&str, &str)], args: &[&str]) -> Output {
    let rooted = under_dot_aspect(files);
    let rooted: Vec<(&str, &str)> = rooted.iter().map(|(p, c)| (p.as_str(), *c)).collect();
    let mut fixture = Fixture::new(&rooted);
    fixture.debug = true;
    fixture.run(args).0
}

/// [`run_in_workspace`] with the paths taken from the workspace root instead,
/// for a fixture that needs a `MODULE.aspect` or a file under `.aspect/lib/`.
fn run_in_rooted_workspace(files: &[(&str, &str)], args: &[&str]) -> Output {
    Fixture::new(files).run(args).0
}

/// Rebase `files` under `.aspect/`.
fn under_dot_aspect<'a>(files: &[(&'a str, &'a str)]) -> Vec<(String, &'a str)> {
    files
        .iter()
        .map(|(name, contents)| (format!(".aspect/{name}"), *contents))
        .collect()
}

/// `output`'s exit code, or a panic naming what it did instead.
fn output_code(output: &Output) -> i32 {
    output
        .status
        .code()
        .unwrap_or_else(|| panic!("killed by a signal: {:?}", output.status))
}

fn stdout(output: &Output) -> String {
    String::from_utf8_lossy(&output.stdout).into_owned()
}

fn stderr(output: &Output) -> String {
    String::from_utf8_lossy(&output.stderr).into_owned()
}

/// The command succeeded, and said on stderr which file failed and why.
fn assert_degraded(output: &Output, file: &str) {
    let stderr = stderr(output);
    assert_eq!(
        output.status.code(),
        Some(0),
        "expected success despite the broken file\n--- stderr ---\n{stderr}"
    );
    assert!(
        stderr.contains("warning: ") && stderr.contains(file),
        "expected a warning naming {file} in:\n{stderr}"
    );
    assert_no_doubled_error_prefix(&stderr);
}

/// The command failed with the AXL diagnostic as its reason.
fn assert_failed_on_axl(output: &Output, needle: &str) {
    let stderr = stderr(output);
    assert_eq!(
        output.status.code(),
        Some(1),
        "expected exit code 1\n--- stderr ---\n{stderr}"
    );
    assert!(stderr.contains(needle), "expected {needle:?} in:\n{stderr}");
    assert_no_doubled_error_prefix(&stderr);
}

/// Starlark renders its diagnostics with their own `error: ` title, so the CLI
/// must not prefix them again — neither adjacently, which `error: error:` is,
/// nor as a second `error:` line further down, which two diagnostics both
/// presented as the run’s own failure look like. A run has one failure, so it
/// gets one `error:`.
///
/// A Starlark traceback carries its own `error:` head line below
/// `Traceback (most recent call last):`, and that head belongs to the
/// diagnostic the traceback is part of rather than being a second failure, so
/// lines inside a traceback block do not count. (The CLI prefixes the
/// traceback’s first line, which leaves two `error:` lines on stderr for the
/// one failure — pre-existing, and not what this checks.) A block is taken to
/// end at the next blank line, which is how Starlark’s renderer ends one
/// rather than anything the CLI promises.
fn assert_no_doubled_error_prefix(stderr: &str) {
    assert!(
        !stderr.contains("error: error:"),
        "the diagnostic was prefixed twice:\n{stderr}"
    );
    let mut in_traceback = false;
    let mut reported: Vec<&str> = Vec::new();
    for line in stderr.lines() {
        if line
            .trim_end()
            .ends_with("Traceback (most recent call last):")
        {
            if line.starts_with("error: ") {
                reported.push(line);
            }
            in_traceback = true;
        } else if line.trim().is_empty() {
            in_traceback = false;
        } else if line.starts_with("error: ") && !in_traceback {
            reported.push(line);
        }
    }
    assert!(
        reported.len() <= 1,
        "a run reports one failure, so one `error:`; got {}: {reported:?}\n{stderr}",
        reported.len()
    );
}

#[test]
fn version_survives_an_unparsable_script() {
    let output = run_in_workspace(&[("policy.axl", UNPARSABLE_SCRIPT)], &["version"]);
    assert_degraded(&output, "policy.axl");
    assert!(
        stderr(&output).contains("Parse error"),
        "the parse error must still be visible:\n{}",
        stderr(&output)
    );
    assert!(
        !stdout(&output).trim().is_empty(),
        "expected a version on stdout"
    );
}

#[test]
fn help_survives_an_unparsable_script_and_lists_what_did_load() {
    let output = run_in_workspace(
        &[("policy.axl", UNPARSABLE_SCRIPT), ("good.axl", GOOD_SCRIPT)],
        &["help"],
    );
    assert_degraded(&output, "policy.axl");
    let stderr = stderr(&output);
    assert!(
        stderr.contains("Parse error"),
        "the parse error must still be visible:\n{stderr}"
    );
    assert!(
        stderr.contains("the tasks it defines are unavailable"),
        "help must say what the broken file cost:\n{stderr}"
    );

    let stdout = stdout(&output);
    for listed in ["build", "test", "goodtask"] {
        assert!(
            stdout.contains(listed),
            "expected {listed:?} in the degraded help:\n{stdout}"
        );
    }
}

/// `--help`, `--version`, a bare `aspect` and a bare `aspect <group>` are
/// rendered by Clap rather than by the `help` / `version` subcommands, so they
/// are their own path.
#[test]
fn clap_rendered_help_survives_an_unparsable_script() {
    let flag = run_in_workspace(&[("policy.axl", UNPARSABLE_SCRIPT)], &["--help"]);
    assert_degraded(&flag, "policy.axl");
    assert!(stdout(&flag).contains("build"), "expected the task list");

    let version = run_in_workspace(&[("policy.axl", UNPARSABLE_SCRIPT)], &["--version"]);
    assert_degraded(&version, "policy.axl");
    assert!(
        !stdout(&version).trim().is_empty(),
        "expected a version on stdout"
    );

    // A bare `aspect` prints the same help, but keeps Clap's "no command"
    // exit code; what matters is that it is not the AXL failure's.
    let bare = run_in_workspace(&[("policy.axl", UNPARSABLE_SCRIPT)], &[]);
    assert_eq!(
        bare.status.code(),
        Some(2),
        "--- stderr ---\n{}",
        stderr(&bare)
    );
    assert!(
        stderr(&bare).contains("warning:") && stderr(&bare).contains("Tasks:"),
        "expected the warning and the help:\n{}",
        stderr(&bare)
    );

    // A bare `aspect <group>` with no leaf is the same Clap error, one level
    // down: `arg_required_else_help` renders the group’s own help.
    // A sample of the built-in groups, which come from `group = [...]` on the
    // built-in tasks; hard-coded rather than derived so the test does not
    // track the shipped AXL.
    for group in ["auth", "worktree", "setup"] {
        let output = run_in_workspace(&[("policy.axl", UNPARSABLE_SCRIPT)], &[group]);
        let stderr = stderr(&output);
        assert_eq!(output.status.code(), Some(2), "`aspect {group}`:\n{stderr}");
        assert!(
            stderr.contains("warning:") && stderr.contains("policy.axl"),
            "`aspect {group}` must still report the broken file:\n{stderr}"
        );
        assert!(
            stderr.contains(&format!("aspect {group}")),
            "`aspect {group}` must print the group’s help:\n{stderr}"
        );
        assert_no_doubled_error_prefix(&stderr);
    }
}

#[test]
fn a_task_still_fails_on_an_unparsable_script() {
    // A built-in, which needs nothing from the broken file, and the task the
    // broken file was supposed to define. Both must fail: the workspace did
    // not fully load, so neither can be trusted to run.
    for args in [vec!["build", "//..."], vec!["policy"]] {
        let output = run_in_workspace(&[("policy.axl", UNPARSABLE_SCRIPT)], &args);
        assert_failed_on_axl(&output, "Parse error");
    }
}

/// `describe` and `feature` render the resolved AXL surface itself, so a
/// surface that quietly lost entries would be a wrong answer rather than a
/// degraded one.
#[test]
fn the_surface_reporting_commands_still_fail_on_an_unparsable_script() {
    for args in [vec!["describe"], vec!["feature"]] {
        let output = run_in_workspace(&[("policy.axl", UNPARSABLE_SCRIPT)], &args);
        assert_failed_on_axl(&output, "Parse error");
    }
}

#[test]
fn version_and_help_survive_a_failing_config() {
    for args in [vec!["version"], vec!["help"]] {
        let output = run_in_workspace(&[("config.axl", FAILING_CONFIG)], &args);
        assert_degraded(&output, "config.axl");
        assert!(
            stderr(&output).contains("no task found"),
            "the config's own error must still be visible:\n{}",
            stderr(&output)
        );
    }
}

#[test]
fn a_task_still_fails_on_a_failing_config() {
    let output = run_in_workspace(&[("config.axl", FAILING_CONFIG)], &["build", "//..."]);
    assert_failed_on_axl(&output, "no task found");
}

/// The warning is a consequence of a broken file, not of every run.
#[test]
fn a_healthy_workspace_warns_about_nothing() {
    let output = run_in_workspace(&[("good.axl", GOOD_SCRIPT)], &["help"]);
    let stderr = stderr(&output);
    assert_eq!(output.status.code(), Some(0), "--- stderr ---\n{stderr}");
    assert!(
        stderr.is_empty(),
        "a workspace that loads must say nothing on stderr:\n{stderr}"
    );
    assert!(
        stdout(&output).contains("goodtask"),
        "expected the user's task in the help"
    );
}

/// Keeps the fixture honest: `tempfile` hands out a path, and a test that
/// silently wrote its `.aspect` somewhere else would pass for the wrong reason.
#[test]
fn the_fixture_workspace_is_where_the_test_thinks_it_is() {
    let output = run_in_workspace(&[("policy.axl", UNPARSABLE_SCRIPT)], &["version"]);
    let stderr = stderr(&output);
    let named = stderr
        .lines()
        .find_map(|line| line.strip_prefix("warning: "))
        .and_then(|line| line.split(" failed; ").next())
        .map(Path::new)
        .expect("a warning naming the file");
    assert!(
        named.is_absolute() && named.ends_with(".aspect/policy.axl"),
        "expected an absolute path to the fixture, got {named:?}"
    );
}

/// An author's `ctx.std.process.exit(code, message)` is a decision, not a file
/// that would not load, so it must end every command with that code and
/// message — a gate saying "you need a newer CLI" matters most on the very
/// command someone runs to check their version. And per the contract in
/// `axl-runtime/src/eval/exit.rs`, no traceback without `ASPECT_DEBUG`.
#[test]
fn a_deliberate_exit_from_a_config_ends_every_command() {
    for args in [
        vec!["version"],
        vec!["help"],
        vec!["--help"],
        vec!["goodtask"],
    ] {
        let output = run_in_workspace(
            &[("config.axl", EXITING_CONFIG), ("good.axl", GOOD_SCRIPT)],
            &args,
        );
        let stderr = stderr(&output);
        let what = args.join(" ");
        assert_eq!(
            output.status.code(),
            Some(3),
            "`aspect {what}` must exit with the author's code\n--- stderr ---\n{stderr}"
        );
        assert!(
            stderr.contains("ERROR: this repo requires a newer aspect CLI"),
            "`aspect {what}` must print the author's message:\n{stderr}"
        );
        assert!(
            !stderr.contains("Traceback"),
            "`aspect {what}` must not print a traceback:\n{stderr}"
        );
        assert!(
            !stderr.contains("warning:"),
            "a deliberate exit is not a load failure to warn about:\n{stderr}"
        );
    }
}

/// The tolerance is per file, not per phase, so phase 2 runs even when phase 1
/// could not load a script and a config that merely refers to a task from the
/// broken file fails too. That second failure is derived, and must not read as
/// an equal, independent problem pointing at a file that is perfectly fine.
#[test]
fn a_config_failure_after_a_script_failure_is_marked_as_derived() {
    let output = run_in_workspace(
        &[
            ("policy.axl", UNPARSABLE_SCRIPT_DEFINING_MYTASK),
            ("config.axl", CONFIG_NEEDING_MYTASK),
        ],
        &["version"],
    );
    assert_degraded(&output, "policy.axl");
    let stderr = stderr(&output);

    assert!(
        stderr.contains("Parse error"),
        "the script's own diagnostic is the one to act on:\n{stderr}"
    );
    assert!(
        stderr.contains("config.axl then failed too, most likely because of")
            && stderr.contains("policy.axl — fix that first"),
        "the derived config failure must say it is derived, and name what of:\n{stderr}"
    );
    assert!(
        !stderr.contains("no task found"),
        "the derived config diagnostic must not be printed as its own warning:\n{stderr}"
    );
    // One real warning for the script; the config gets the summary line, not
    // a second `warning: <file> failed` of equal weight.
    assert_eq!(
        stderr
            .lines()
            .filter(|l| l.starts_with("warning:") && l.contains(" failed; "))
            .count(),
        1,
        "expected exactly one load warning:\n{stderr}"
    );
}

/// The same derived config on a command that dispatches a task. The script is
/// raised as the run’s `error:` and so prints *below* the summary line, which
/// is why that line names the script it blames instead of saying "the above".
#[test]
fn a_derived_config_names_its_cause_when_the_command_cannot_run() {
    let output = run_in_workspace(
        &[
            ("policy.axl", UNPARSABLE_SCRIPT_DEFINING_MYTASK),
            ("config.axl", CONFIG_NEEDING_MYTASK),
        ],
        &["build", "//..."],
    );
    assert_failed_on_axl(&output, "Parse error");
    let stderr = stderr(&output);
    assert!(
        stderr.contains("config.axl then failed too, most likely because of")
            && stderr.contains("policy.axl — fix that first"),
        "the summary must name the script it blames:\n{stderr}"
    );
    assert_eq!(
        stderr
            .lines()
            .filter(|l| l.starts_with("warning:") && l.contains(" failed; "))
            .count(),
        0,
        "the derived config must not be promoted to a warning of its own:\n{stderr}"
    );
    assert!(
        !stderr.contains("no task found"),
        "nor have its own diagnostic printed:\n{stderr}"
    );
}

/// Which of several broken scripts the summary blames, on a command that
/// cannot run: the one raised as the run’s `error:`. That is the one printed
/// *below* the summary, so it is the one a reference to "the above" would
/// miss, and pointing the user at a different file than the error they are
/// looking at would be worse than vague.
///
/// Two files hold the same broken script, and which of them `read_dir` hands
/// over first — and so which is raised — is not the test’s to assume: it
/// reads the blamed path out of the summary and checks that the diagnostic
/// below names that same file, with the other script’s warning above.
#[test]
fn the_summary_blames_the_script_the_run_raises() {
    let output = run_in_workspace(
        &[
            ("apolicy.axl", UNPARSABLE_SCRIPT_DEFINING_MYTASK),
            ("zpolicy.axl", UNPARSABLE_SCRIPT_DEFINING_MYTASK),
            ("config.axl", CONFIG_NEEDING_MYTASK),
        ],
        &["build", "//..."],
    );
    assert_failed_on_axl(&output, "Parse error");
    let stderr = stderr(&output);

    let summary = stderr
        .lines()
        .find(|line| line.contains("then failed too, most likely because of"))
        .unwrap_or_else(|| panic!("expected a derived-config summary in:\n{stderr}"));
    let blamed = summary
        .split_once("most likely because of ")
        .and_then(|(_, tail)| tail.strip_suffix(" — fix that first"))
        .unwrap_or_else(|| panic!("the summary must name one file: {summary:?}"));
    let (above, below) = stderr
        .split_once(summary)
        .expect("the summary line came out of this stderr");

    assert!(
        below.contains(blamed),
        "the summary blames {blamed}, which must be the file the error below names:\n{stderr}"
    );
    let unraised = ["apolicy.axl", "zpolicy.axl"]
        .into_iter()
        .find(|name| !blamed.ends_with(name))
        .expect("one of the two scripts is not the blamed one");
    assert!(
        above.contains(unraised) && !below.contains(unraised),
        "the script that was not raised belongs above, as a warning of its own:\n{stderr}"
    );
}

/// With several scripts broken and none of them raised, no cause is named.
/// Which one the config needed is not something the CLI knows, so naming one
/// would be publishing `read_dir`’s order as a finding — and every one of
/// them is above, which is what the summary points at instead.
#[test]
fn a_derived_config_points_at_the_failures_above_when_several_scripts_failed() {
    let output = run_in_workspace(
        &[
            ("policy.axl", UNPARSABLE_SCRIPT_DEFINING_MYTASK),
            ("other.axl", UNPARSABLE_SCRIPT),
            ("config.axl", CONFIG_NEEDING_MYTASK),
        ],
        &["version"],
    );
    assert_degraded(&output, "policy.axl");
    let stderr = stderr(&output);

    assert!(
        stderr.contains(
            "config.axl then failed too, most likely because of the failures above — fix those \
             first"
        ),
        "the summary must point at the failures above, plural:\n{stderr}"
    );
    assert!(
        !stderr.contains("fix that first"),
        "no single script may be blamed when several failed:\n{stderr}"
    );
    // Both scripts are above the summary, each with its own warning.
    assert_eq!(
        stderr
            .lines()
            .filter(|l| l.starts_with("warning:") && l.contains(" failed; "))
            .count(),
        2,
        "expected a load warning per broken script:\n{stderr}"
    );
}

/// Every config that failed is named, and all of them share the one summary
/// line. They were all derived from the same broken script, so a `warning:`
/// each would read as that many independent problems; dropping all but the
/// first would instead leave a user editing a file the warning never
/// mentioned.
#[test]
fn the_summary_names_every_config_that_failed() {
    // The workspace's own config and the user-global one, which is why the
    // paths are rooted rather than rebased under `.aspect/`: `home/` is the
    // run's throwaway `$HOME`.
    let files = [
        (".aspect/apolicy.axl", UNPARSABLE_SCRIPT_DEFINING_MYTASK),
        (".aspect/config.axl", CONFIG_NEEDING_MYTASK),
        ("home/.aspect/config.axl", CONFIG_NEEDING_MYTASK),
    ];
    let output = run_in_rooted_workspace(&files, &["version"]);
    assert_degraded(&output, "apolicy.axl");
    let stderr = stderr(&output);

    let summary = stderr
        .lines()
        .find(|line| line.contains("then failed too, most likely because of"))
        .unwrap_or_else(|| panic!("expected a derived-config summary in:\n{stderr}"));
    // The paths are absolute, so each is recognised by its tail — and the
    // workspace config's tail is itself a suffix of the user-global one's,
    // which is why the list is split on the join before anything is matched.
    let listed: Vec<&str> = summary
        .trim_start_matches("warning: ")
        .split_once(" then failed too")
        .map(|(paths, _)| paths.split(", ").collect())
        .unwrap_or_else(|| panic!("the summary must list the configs: {summary:?}"));

    assert_eq!(
        listed.len(),
        2,
        "both failed configs belong in the one summary line: {summary:?}"
    );
    assert!(
        listed[0].ends_with("/.aspect/config.axl")
            && !listed[0].ends_with("home/.aspect/config.axl"),
        "the workspace config is named first: {summary:?}"
    );
    assert!(
        listed[1].ends_with("home/.aspect/config.axl"),
        "the user-global config runs last among configs, and is reported last: \
         {summary:?}"
    );
    assert!(
        summary.ends_with("apolicy.axl — fix that first"),
        "one script failed, so it is the cause both configs are blamed on: {summary:?}"
    );
    // One warning for the script, and none for either config.
    assert_eq!(
        stderr
            .lines()
            .filter(|l| l.starts_with("warning:") && l.contains(" failed; "))
            .count(),
        1,
        "expected exactly one load warning:\n{stderr}"
    );
}

/// Every broken script is named, and scripts are reported before configs so
/// the cause precedes the consequence.
#[test]
fn every_broken_script_is_reported_before_any_config() {
    let output = run_in_workspace(
        &[
            ("aaa.axl", UNPARSABLE_SCRIPT),
            ("zzz.axl", UNPARSABLE_SCRIPT),
            ("config.axl", CONFIG_NEEDING_MYTASK),
        ],
        &["version"],
    );
    assert_eq!(output.status.code(), Some(0), "{}", stderr(&output));
    let stderr = stderr(&output);
    for named in ["aaa.axl", "zzz.axl", "config.axl"] {
        assert!(
            stderr.contains(named),
            "expected {named} named in:\n{stderr}"
        );
    }
    let last_script = ["aaa.axl", "zzz.axl"]
        .iter()
        .map(|f| stderr.find(f).expect("script named"))
        .max()
        .expect("two scripts");
    let config = stderr
        .find("config.axl then failed too")
        .expect("the derived line");
    assert!(
        last_script < config,
        "scripts must be reported before the config they broke:\n{stderr}"
    );
}

/// `use_task(path, "good", "notatask")` names one symbol that exists and one
/// that does not. Registration is all-or-nothing, so neither lands: a `help`
/// that warned the file is unavailable must not then list a task out of it,
/// which `aspect <task>` would refuse to run.
#[test]
fn a_partial_use_task_registers_none_of_its_tasks() {
    const PAIR: &str = r#"
def _impl(ctx: TaskContext) -> int:
    return 0

firsttask = task(summary = "the first of a pair.", implementation = _impl)
"#;
    let files = [
        (
            "MODULE.aspect",
            "use_task(\".aspect/lib/pair.axl\", \"firsttask\", \"notatask\")\n",
        ),
        (".aspect/version.axl", "version(\"0.0.0-dev\")\n"),
        (".aspect/lib/pair.axl", PAIR),
    ];

    let help = run_in_rooted_workspace(&files, &["help"]);
    assert_degraded(&help, "pair.axl");
    assert!(
        !stdout(&help).contains("firsttask"),
        "a file reported unavailable must contribute no tasks:\n{}",
        stdout(&help)
    );

    let run = run_in_rooted_workspace(&files, &["firsttask"]);
    assert_failed_on_axl(&run, "notatask");
}

/// The short spellings go through the same Clap path as `--help`/`--version`.
#[test]
fn short_help_and_version_flags_survive_an_unparsable_script() {
    for flag in ["-h", "-v"] {
        let output = run_in_workspace(&[("policy.axl", UNPARSABLE_SCRIPT)], &[flag]);
        assert_degraded(&output, "policy.axl");
        assert!(
            !stdout(&output).trim().is_empty(),
            "`aspect {flag}` must still print something"
        );
    }
}

/// A task's own `--help`, and a group named without a leaf, print help and run
/// nothing, so they degrade like the root help does. The group keeps Clap's
/// "no command" exit code; what matters is that it is not the AXL failure's.
#[test]
fn a_tasks_own_help_survives_an_unparsable_script() {
    let task_help = run_in_workspace(&[("policy.axl", UNPARSABLE_SCRIPT)], &["build", "--help"]);
    assert_degraded(&task_help, "policy.axl");
    assert!(
        stdout(&task_help).contains("Build Bazel targets"),
        "expected `build`'s own help:\n{}",
        stdout(&task_help)
    );

    let group = run_in_workspace(&[("policy.axl", UNPARSABLE_SCRIPT)], &["auth"]);
    assert_eq!(
        group.status.code(),
        Some(2),
        "--- stderr ---\n{}",
        stderr(&group)
    );
    assert!(
        stderr(&group).contains("warning:"),
        "expected the warning alongside the group help:\n{}",
        stderr(&group)
    );
    assert_no_doubled_error_prefix(&stderr(&group));
}

/// An error that ends the run does not excuse dropping the typos recorded on
/// the way to it: the gate is what to act on, but the typo is still a typo,
/// and losing it is the silence this tolerance exists to avoid.
#[test]
fn a_file_failure_recorded_before_a_gate_is_still_reported() {
    // Phase 2's gate, with a phase-1 failure already on the books.
    let config_gate = run_in_workspace(
        &[
            ("broken.axl", UNPARSABLE_SCRIPT),
            ("config.axl", EXITING_CONFIG),
        ],
        &["help"],
    );
    let stderr = stderr(&config_gate);
    assert_eq!(output_code(&config_gate), 3, "--- stderr ---\n{stderr}");
    assert!(
        stderr.contains("broken.axl") && stderr.contains("Parse error"),
        "the typo must survive the gate:\n{stderr}"
    );
    assert!(
        stderr.contains("ERROR: this repo requires a newer aspect CLI"),
        "and so must the gate:\n{stderr}"
    );

    // Phase 1's own gate: an error type declared `traceback = False`, raised
    // from a script body. Phase 1 finishes loading before it raises, so both
    // typos are reported whatever order the scripts were discovered in.
    //
    // The gate is written FIRST on purpose. Scripts are discovered with
    // `read_dir`, which on the filesystems we test on returns them in creation
    // order, so writing the gate first is what makes this fixture discriminate:
    // a phase that stopped at the gate would report neither typo. Written last,
    // it would pass either way.
    const SCRIPT_GATE: &str = r#"
Gate = error.type(traceback = False)

fail(Gate("this checkout needs a newer CLI"))
"#;
    let script_gate = run_in_workspace(
        &[
            ("zzz.axl", SCRIPT_GATE),
            ("aaa.axl", UNPARSABLE_SCRIPT),
            ("mmm.axl", UNPARSABLE_SCRIPT),
        ],
        &["help"],
    );
    let stderr = self::stderr(&script_gate);
    assert_eq!(output_code(&script_gate), 1, "--- stderr ---\n{stderr}");
    assert_eq!(
        stderr
            .lines()
            .filter(|l| l.starts_with("warning:") && l.contains(" failed; "))
            .count(),
        2,
        "both typos must be reported, in any directory order:\n{stderr}"
    );
    assert!(
        stderr.contains("ERROR: this checkout needs a newer CLI"),
        "the gate still ends the run:\n{stderr}"
    );
    assert!(
        !stderr.contains("Traceback"),
        "a `traceback = False` gate prints no traceback:\n{stderr}"
    );
}

/// The derived config diagnostic is suppressed, not lost: `ASPECT_DEBUG`
/// prints it, for when the guess that it was derived turns out wrong.
#[test]
fn aspect_debug_prints_the_suppressed_config_diagnostic() {
    let files = [
        ("policy.axl", UNPARSABLE_SCRIPT_DEFINING_MYTASK),
        ("config.axl", CONFIG_NEEDING_MYTASK),
    ];
    let quiet = run_in_workspace(&files, &["version"]);
    assert_degraded(&quiet, "policy.axl");
    assert!(
        !stderr(&quiet).contains("no task found"),
        "suppressed by default:\n{}",
        stderr(&quiet)
    );

    let debug = run_in_debug_workspace(&files, &["version"]);
    assert_eq!(debug.status.code(), Some(0), "{}", stderr(&debug));
    assert!(
        stderr(&debug).contains("no task found"),
        "ASPECT_DEBUG must print it:\n{}",
        stderr(&debug)
    );
}

/// A config that ends the run stops the configs after it. A config body has a
/// `ctx` and can write files and send requests, so running more of them past
/// an author's `exit()` would be doing the work that exit exists to prevent.
///
/// `search_sources` finds at most one `config.axl` per directory, so a second
/// config means a nested `.aspect/` plus a cwd at or below it.
#[test]
fn a_config_gate_stops_the_configs_after_it() {
    const WRITES_A_MARKER: &str = r#"
def config(ctx):
    ctx.std.fs.write_file("ran", "yes")
"#;
    let files = [
        (".aspect/config.axl", EXITING_CONFIG),
        ("sub/.aspect/config.axl", WRITES_A_MARKER),
    ];
    let mut fixture = Fixture::new(&files);
    fixture.cwd = "sub";
    let (output, dir) = fixture.run(&["version"]);

    let stderr = stderr(&output);
    assert_eq!(
        output_code(&output),
        3,
        "the gate must end the run\n--- stderr ---\n{stderr}"
    );
    assert!(
        !dir.path().join("sub/ran").exists(),
        "the config after the gate must not have run:\n{stderr}"
    );
    // Nor may it be reported: it never ran, so it has nothing to say.
    assert!(
        !stderr.contains("sub"),
        "a config that never ran must not be named:\n{stderr}"
    );
    dir.close().expect("removing temp dir");
}

/// An ordinary config failure is recorded and the remaining configs still run,
/// so one typo does not hide the next. The inverse of
/// `a_config_gate_stops_the_configs_after_it`, and the pair pins the polarity
/// of what `record` returns.
#[test]
fn every_broken_config_is_reported() {
    let files = [
        (
            ".aspect/config.axl",
            "def config(ctx):\n    ctx.tasks[\"nope-one\"].args.x = 1\n",
        ),
        (
            "sub/.aspect/config.axl",
            "def config(ctx):\n    ctx.tasks[\"nope-two\"].args.x = 1\n",
        ),
    ];
    let mut fixture = Fixture::new(&files);
    fixture.cwd = "sub";
    let (output, dir) = fixture.run(&["version"]);

    let stderr = stderr(&output);
    assert_eq!(output_code(&output), 0, "--- stderr ---\n{stderr}");
    for named in ["nope-one", "nope-two"] {
        assert!(
            stderr.contains(named),
            "expected {named}'s config to be reported too:\n{stderr}"
        );
    }
    assert_eq!(
        stderr.lines().filter(|l| l.starts_with("warning:")).count(),
        2,
        "expected one warning per broken config:\n{stderr}"
    );
    dir.close().expect("removing temp dir");
}

/// A task-name conflict is judged on the finished surface, after both phases,
/// so it is a raise path like any other and has to report what did not load on
/// its way out.
#[test]
fn a_name_conflict_still_reports_what_did_not_load() {
    const RESERVED_NAME: &str = r#"
def _impl(ctx: TaskContext) -> int:
    return 0

describe_task = task(kind = "describe", summary = "clashes with a command.", implementation = _impl)
"#;
    const DUPLICATE_NAME: &str = r#"
def _impl(ctx: TaskContext) -> int:
    return 0

one = task(kind = "dup", summary = "one.", implementation = _impl)
two = task(kind = "dup", summary = "two.", implementation = _impl)
"#;
    for clashing in [RESERVED_NAME, DUPLICATE_NAME] {
        let output = run_in_workspace(
            &[("broken.axl", UNPARSABLE_SCRIPT), ("clash.axl", clashing)],
            &["help"],
        );
        let stderr = stderr(&output);
        assert_eq!(output_code(&output), 1, "--- stderr ---\n{stderr}");
        assert!(
            stderr.contains("broken.axl") && stderr.contains("Parse error"),
            "the typo must survive the conflict:\n{stderr}"
        );
        assert_no_doubled_error_prefix(&stderr);
    }
}

/// A command that cannot run degraded still reports *every* file that failed,
/// not just the one it raises. Scripts are discovered with `read_dir`, so
/// dropping the rest would make the diagnostics a user sees depend on
/// directory order — two typos, and which one you hear about is luck.
///
/// Both refusal paths are covered: a built-in that parses fine and would
/// dispatch, and an unknown subcommand, which Clap rejects against a surface
/// built from a workspace that did not fully load.
#[test]
fn every_broken_script_is_reported_even_when_the_command_cannot_run() {
    for args in [vec!["build", "//..."], vec!["notacommand"]] {
        let output = run_in_workspace(
            &[
                ("aaa.axl", UNPARSABLE_SCRIPT),
                ("zzz.axl", UNPARSABLE_SCRIPT),
            ],
            &args,
        );
        let stderr = stderr(&output);
        let what = args.join(" ");
        assert_eq!(
            output_code(&output),
            1,
            "`aspect {what}`\n--- stderr ---\n{stderr}"
        );
        for named in ["aaa.axl", "zzz.axl"] {
            assert!(
                stderr.contains(named),
                "`aspect {what}` must name {named} too:\n{stderr}"
            );
        }
        // One is raised as the run's error, the rest come as warnings — so
        // the raised one is not also warned about, and nothing is lost.
        assert_eq!(
            stderr
                .lines()
                .filter(|l| l.starts_with("warning:") && l.contains(" failed; "))
                .count(),
            1,
            "`aspect {what}`: one raised, one warned:\n{stderr}"
        );
        assert_no_doubled_error_prefix(&stderr);
    }
}

/// A `use_feature` file defines no tasks, so saying its tasks went missing
/// would send the user looking for something that was never there. What it
/// actually costs is the feature and the flags that feature adds to every task
/// subcommand.
#[test]
fn a_broken_feature_file_is_reported_as_a_feature() {
    let files = [
        (
            "MODULE.aspect",
            "use_feature(\".aspect/lib/feat.axl\", \"afeature\")\n",
        ),
        (".aspect/version.axl", "version(\"0.0.0-dev\")\n"),
        (".aspect/lib/feat.axl", UNPARSABLE_SCRIPT),
    ];
    let output = run_in_rooted_workspace(&files, &["help"]);
    assert_degraded(&output, "feat.axl");
    let stderr = stderr(&output);
    assert!(
        stderr.contains("the feature it defines is unavailable"),
        "a broken feature file must be reported as a feature:\n{stderr}"
    );
    assert!(
        !stderr.contains("the tasks it defines"),
        "it defines no tasks:\n{stderr}"
    );
    assert!(
        stderr.contains("flags it adds to every task"),
        "a lost feature takes its flags with it:\n{stderr}"
    );
}

/// A `MODULE.aspect` may name one file twice — `use_task` and `use_feature` on
/// the same path. It fails identically each time, so the user hears about it
/// once, with both roles folded into what its absence costs.
#[test]
fn a_file_referenced_twice_is_reported_once() {
    let files = [
        (
            "MODULE.aspect",
            "use_task(\".aspect/lib/both.axl\", \"atask\")\n\
             use_feature(\".aspect/lib/both.axl\", \"afeature\")\n",
        ),
        (".aspect/version.axl", "version(\"0.0.0-dev\")\n"),
        (".aspect/lib/both.axl", UNPARSABLE_SCRIPT),
    ];
    let output = run_in_rooted_workspace(&files, &["help"]);
    assert_degraded(&output, "both.axl");
    let stderr = stderr(&output);
    assert_eq!(
        stderr
            .lines()
            .filter(|l| l.starts_with("warning:") && l.contains(" failed; "))
            .count(),
        1,
        "one file, one warning:\n{stderr}"
    );
    assert_eq!(
        stderr.matches("Parse error").count(),
        1,
        "and its diagnostic printed once:\n{stderr}"
    );
    assert!(
        stderr.contains("the tasks and the feature it defines are unavailable"),
        "both roles must be named:\n{stderr}"
    );
}

/// A `MODULE.aspect` that cannot be *read* is fatal the same way, and must
/// still say which file it was: it is the one file opened before the tolerance
/// begins, so nothing else on stderr names it.
#[test]
#[cfg(unix)]
fn an_unreadable_module_aspect_names_itself() {
    for args in [vec!["help"], vec!["version"], vec!["build", "//..."]] {
        let mut fixture = Fixture::new(&[
            ("MODULE.aspect", "aspect(name = \"unreadable\")\n"),
            (".aspect/version.axl", "version(\"0.0.0-dev\")\n"),
        ]);
        fixture.unreadable = &["MODULE.aspect"];
        let output = fixture.run(&args).0;
        let stderr = stderr(&output);
        let what = args.join(" ");
        assert_eq!(output_code(&output), 1, "`aspect {what}`:\n{stderr}");
        assert!(
            stderr.contains("MODULE.aspect:") && stderr.contains("Permission denied"),
            "`aspect {what}` must name the file it could not read:\n{stderr}"
        );
        assert_no_doubled_error_prefix(&stderr);
    }
}

/// A `MODULE.aspect` is read before either tolerant phase, so one that does
/// not parse is fatal to every command — and there is nothing recorded yet to
/// warn about, the broken script beside it never having been reached.
#[test]
fn an_unparsable_module_aspect_is_fatal_and_warns_about_nothing() {
    for args in [vec!["help"], vec!["version"], vec!["build", "//..."]] {
        let output = run_in_rooted_workspace(
            &[
                ("MODULE.aspect", UNPARSABLE_SCRIPT),
                (".aspect/broken.axl", UNPARSABLE_SCRIPT),
            ],
            &args,
        );
        let stderr = stderr(&output);
        let what = args.join(" ");
        assert_eq!(
            output_code(&output),
            1,
            "`aspect {what}`\n--- stderr ---\n{stderr}"
        );
        assert!(
            stderr.contains("MODULE.aspect") && stderr.contains("Parse error"),
            "`aspect {what}` must report the MODULE.aspect:\n{stderr}"
        );
        assert!(
            !stderr.contains("warning:"),
            "nothing had been loaded yet, so there is nothing to warn about:\n{stderr}"
        );
        assert_no_doubled_error_prefix(&stderr);
    }
}

/// A trait field a config left holding the wrong type is judged once every
/// config has run, so it is settled outside the per-file tolerance and fails
/// every command — and like every other raise path it reports what did not
/// load on its way out.
#[test]
fn a_trait_type_mismatch_still_reports_what_did_not_load() {
    const DECLARES_A_TRAIT: &str = r#"
MyTrait = trait(scores = attr(list[int], default = []))

def _impl(ctx: TaskContext) -> int:
    return 0

traittask = task(summary = "Uses a trait.", traits = [MyTrait], implementation = _impl)
"#;
    // Assignment is type-checked as it happens; a list mutated in place is
    // not, which is why the check runs once at the end of phase 2.
    const APPENDS_THE_WRONG_TYPE: &str = r#"
load("./traits.axl", "MyTrait")

def config(ctx):
    ctx.traits[MyTrait].scores.append("not an int")
"#;
    let output = run_in_workspace(
        &[
            ("broken.axl", UNPARSABLE_SCRIPT),
            ("traits.axl", DECLARES_A_TRAIT),
            ("config.axl", APPENDS_THE_WRONG_TYPE),
        ],
        &["help"],
    );
    let stderr = stderr(&output);
    assert_eq!(output_code(&output), 1, "--- stderr ---\n{stderr}");
    assert!(
        stderr.contains("broken.axl") && stderr.contains("Parse error"),
        "the typo must survive the mismatch:\n{stderr}"
    );
    assert!(
        stderr.contains("expected type `list[int]`"),
        "and the mismatch is still what ends the run:\n{stderr}"
    );
    assert_no_doubled_error_prefix(&stderr);
}

/// The caveat on a degraded `help`: phase 2 mutates the shared heap as it
/// goes, so a task a config registered before failing is still in the listing,
/// even though running it hits the same refusal every other task does. Best
/// effort, not a promise of runnability.
#[test]
fn a_task_a_failed_config_added_stays_in_the_degraded_help() {
    const DEFINES_THE_GHOST: &str = r#"
def _impl(ctx: TaskContext) -> int:
    return 0

ghosttask = task(summary = "Added by a config that then failed.", implementation = _impl)
"#;
    const ADDS_IT_THEN_FAILS: &str = r#"
load("./lib/ghost.axl", "ghosttask")

def config(ctx):
    ctx.tasks.add(ghosttask)
    ctx.tasks["no-such-task"].args.anything = 1
"#;
    // Under `lib/`, so phase 1 does not discover it and the config's
    // `ctx.tasks.add` is what puts it on the surface.
    let files = [
        (".aspect/lib/ghost.axl", DEFINES_THE_GHOST),
        (".aspect/config.axl", ADDS_IT_THEN_FAILS),
    ];

    let help = run_in_rooted_workspace(&files, &["help"]);
    assert_degraded(&help, "config.axl");
    assert!(
        stdout(&help).contains("ghosttask"),
        "the ghost is listed — the mutation happened before the failure:\n{}",
        stdout(&help)
    );

    let run = run_in_rooted_workspace(&files, &["ghosttask"]);
    assert_failed_on_axl(&run, "no task found");
}

/// `~/.aspect/config.axl` evaluates in its own module scope, which is a
/// separate path through phase 2. It is tolerated and reported like any other
/// config — the user who broke their own global config needs `aspect help`
/// just as much, and in every repo they walk into.
#[test]
fn a_broken_user_global_config_is_reported_like_any_other() {
    // `home/` is the throwaway `$HOME` every fixture run gets.
    const BROKEN_USER_CONFIG: &str = r#"
def config(ctx):
    ctx.tasks["no-such-user-task"].args.x = 1
"#;
    let files = [("home/.aspect/config.axl", BROKEN_USER_CONFIG)];

    let degraded = run_in_rooted_workspace(&files, &["version"]);
    assert_degraded(&degraded, "home/.aspect/config.axl");
    assert!(
        stderr(&degraded).contains("no-such-user-task"),
        "the global config's own error must be visible:\n{}",
        stderr(&degraded)
    );

    let dispatched = run_in_rooted_workspace(&files, &["build", "//..."]);
    assert_failed_on_axl(&dispatched, "no-such-user-task");
}
