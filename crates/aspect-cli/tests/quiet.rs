//! Quiet mode changes terminal presentation while preserving task behavior.
//! Spawn the real CLI so discovery, freezing, CLI overrides, lifecycle hooks,
//! and the runtime's opening and closing output are exercised together.

use std::path::PathBuf;
use std::process::{Command, Output};

const TASKS: &str = r#"
load("@aspect//private/lib/environment.axl", "error", "warn")
load("@aspect//private/lib/health_check.axl", "HealthCheckTrait", "run_health_checks")
load("@aspect//private/lib/lifecycle.axl", "Phase", "Phases", "phases")

def _impl(ctx):
    ctx.defer(ctx.std.io.stdout.write, "cleanup\n")
    ctx.std.io.stdout.write("task stdout\n")
    print("task stderr")
    warn(ctx.std, "task warning")
    ph = phases.new(ctx, {})
    ph.update("running", "phase progress", phase = Phase(name = "work"))
    ph.update("running", "same phase progress", phase = Phase(name = "work"))
    ph.update("running", "heartbeat progress")
    if ctx.args.hard_error:
        fail("task hard error")
    if ctx.args.code:
        error(ctx.std, "task error")
    return ph.update(
        "failed" if ctx.args.code else "passed",
        "final progress",
        phase = Phase(name = "finish"),
        final = True,
        conclusion = "task conclusion",
        exit_code = ctx.args.code,
    )

check = task(
    implementation = _impl,
    summary = "Exercise quiet task execution.",
    quiet = True,
    traits = [Phases],
    args = {
        "code": args.int(default = 0),
        "hard_error": args.boolean(default = False),
    },
)

# Alias metadata defaults independently of the base task.
normal = check.alias(summary = "Exercise normal task execution.")

def _runner_check(ctx):
    def health_check():
        ctx.std.io.stdout.write("health checked\n")
        return struct(
            outcome = "unhealthy" if ctx.args.code else "healthy",
            message = "test health failure" if ctx.args.code else "",
        )

    # Exercise the registered Workflows hooks with real task metadata and I/O,
    # replacing only the Bazel server probe.
    hook_ctx = struct(
        task = ctx.task,
        std = ctx.std,
        bazel = struct(
            active_rc = lambda: struct(startup_flags = lambda: ["--output_base=/tmp/quiet-test"]),
            health_check = health_check,
        ),
    )
    result = run_health_checks(hook_ctx, ctx.traits[HealthCheckTrait])
    if result != None:
        error(ctx.std, result)
        return 7
    return 0

runner_check = task(
    implementation = _runner_check,
    summary = "Exercise Workflows setup diagnostics.",
    quiet = True,
    traits = [HealthCheckTrait],
    args = {"code": args.int(default = 0)},
)
"#;

const CONFIG: &str = r#"
load("@aspect//feature/buildkite_annotations.axl", "BuildkiteAnnotations")
load("@aspect//private/lib/lifecycle.axl", "Phases")

def _observe(ctx, update):
    ctx.std.io.stdout.write("event:%s:%s:%s:%s:%s\n" % (
        ctx.task.current_phase().name,
        update.phase_change,
        update.status,
        update.final,
        update.conclusion,
    ))

def config(ctx):
    ctx.features[BuildkiteAnnotations].args.enabled = False
    ctx.traits[Phases].task_update.append(_observe)
"#;

struct Fixture {
    root: tempfile::TempDir,
    bin: PathBuf,
}

impl Fixture {
    fn new() -> Self {
        let bin = std::env::var("ASPECT_CLI_BIN")
            .ok()
            .or_else(|| option_env!("CARGO_BIN_EXE_aspect-cli").map(str::to_owned))
            .expect("set ASPECT_CLI_BIN or run under cargo");
        // Resolve Bazel's runfiles-relative path before changing the child cwd.
        let bin = std::fs::canonicalize(bin).expect("CLI binary exists");
        let root = tempfile::tempdir().unwrap();
        std::fs::create_dir(root.path().join(".aspect")).unwrap();
        std::fs::create_dir(root.path().join("home")).unwrap();
        std::fs::write(root.path().join("MODULE.aspect"), "").unwrap();
        std::fs::write(root.path().join(".aspect/tasks.axl"), TASKS).unwrap();
        std::fs::write(root.path().join(".aspect/config.axl"), CONFIG).unwrap();
        Self { root, bin }
    }

    fn run(&self, args: &[&str], buildkite: bool) -> Output {
        self.command(args, buildkite).output().expect("spawn CLI")
    }

    fn command(&self, args: &[&str], buildkite: bool) -> Command {
        let mut cmd = Command::new(&self.bin);
        // No user config, exporters, or inherited CI settings in this fixture.
        cmd.env_clear()
            .env("HOME", self.root.path().join("home"))
            .env("ASPECT_CLI_CACHE", self.root.path().join("cache"))
            .current_dir(self.root.path())
            .args(args);
        if buildkite {
            cmd.env("BUILDKITE", "true");
        }
        cmd
    }
}

fn assert_presentation(stderr: &str, quiet: bool, buildkite: bool, completed: bool) {
    assert!(stderr.contains("task stderr"), "{stderr}");
    assert!(stderr.contains("task warning"), "{stderr}");
    for text in ["Running", "phase progress", "heartbeat progress"] {
        assert_eq!(stderr.contains(text), !quiet, "{text}: {stderr}");
    }
    if completed {
        assert_eq!(stderr.contains("task conclusion"), !quiet, "{stderr}");
    }
    assert_eq!(
        stderr.lines().any(|line| line.starts_with("--- ")),
        buildkite && !quiet,
        "{stderr}"
    );
    if quiet {
        assert!(!stderr.contains('→'), "{stderr}");
        assert!(!stderr.contains("^^^ +++"), "{stderr}");
    }
}

#[test]
fn quiet_defaults_and_overrides_preserve_output_events_and_exit_codes() {
    let fixture = Fixture::new();
    for buildkite in [false, true] {
        for code in ["0", "7"] {
            // Exercise both default values and overrides, including a global
            // flag before the task and an explicit false after it.
            for (args, quiet) in [
                (vec!["check"], true),
                (vec!["check", "--task:quiet=false"], false),
                (vec!["normal"], false),
                (vec!["--task:quiet", "normal"], true),
            ] {
                let mut args = args;
                args.extend(["--code", code]);
                let output = fixture.run(&args, buildkite);
                let stderr = String::from_utf8_lossy(&output.stderr);
                assert_eq!(
                    output.status.code(),
                    Some(code.parse().unwrap()),
                    "{stderr}"
                );
                let status = if code == "0" { "passed" } else { "failed" };
                assert_eq!(
                    String::from_utf8_lossy(&output.stdout),
                    format!(
                        "task stdout\nevent:work:True:running:False:\n\
                         event:work:False:running:False:\n\
                         event:work:False:running:False:\n\
                         event:finish:True:{status}:True:task conclusion\ncleanup\n"
                    ),
                    "{stderr}"
                );
                assert_presentation(&stderr, quiet, buildkite, true);
                assert_eq!(stderr.contains("task error"), code != "0", "{stderr}");
            }
        }
    }
}

#[test]
fn quiet_preserves_hard_errors_and_deferred_cleanup() {
    let fixture = Fixture::new();
    for (args, quiet) in [
        (vec!["check", "--hard-error"], true),
        (vec!["check", "--hard-error", "--task:quiet=false"], false),
    ] {
        let output = fixture.run(&args, false);
        let stderr = String::from_utf8_lossy(&output.stderr);
        assert_eq!(output.status.code(), Some(1), "{stderr}");
        assert!(stderr.contains("task hard error"), "{stderr}");
        assert!(output.stdout.ends_with(b"cleanup\n"), "{stderr}");
        assert_presentation(&stderr, quiet, false, false);
    }
}

#[test]
fn quiet_preserves_workflows_diagnostics_without_buildkite_sections() {
    let fixture = Fixture::new();
    for code in ["0", "7"] {
        for quiet in [true, false] {
            let mut args = vec!["runner-check", "--code", code];
            if !quiet {
                args.push("--task:quiet=false");
            }
            let output = fixture
                .command(&args, true)
                .env("BUILDKITE_REPO", "https://example.invalid/test/repo.git")
                .env("ASPECT_WORKFLOWS_RUNNER", "1")
                .env("ASPECT_WORKFLOWS_RUNNER_NO_LEGACY_CLI", "1")
                .env("ASPECT_WORKFLOWS_RUNNER_INSTANCE_ID", "quiet-test-runner")
                .env(
                    "ASPECT_WORKFLOWS_RUNNER_BIN_DIR",
                    fixture.root.path().join("bin"),
                )
                .output()
                .expect("spawn CLI");
            let stderr = String::from_utf8_lossy(&output.stderr);
            assert_eq!(
                output.status.code(),
                Some(code.parse().unwrap()),
                "{stderr}"
            );
            assert_eq!(output.stdout, b"health checked\n", "{stderr}");
            for text in [
                "Workflows runner metadata",
                "quiet-test-runner",
                "Runner Health",
                "Bazel Health",
                if code == "0" {
                    "bazel health check passed"
                } else {
                    "bazel health check failed: test health failure"
                },
            ] {
                assert!(stderr.contains(text), "{text}: {stderr}");
            }
            for marker in ["--- :rocket:", "--- :thermometer:"] {
                assert_eq!(stderr.contains(marker), !quiet, "{marker}: {stderr}");
            }
            if quiet {
                assert!(
                    !stderr.lines().any(|line| line.starts_with("--- ")),
                    "{stderr}"
                );
            }
        }
    }
}
