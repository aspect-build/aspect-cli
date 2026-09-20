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
//!
//! Every run gets `stub_bazel_dir()` at the front of its PATH. The command asks
//! Bazel its version, and the Workflows feature probes for the legacy CLI, so a
//! real `bazel` on PATH means `bazel info` starting a server per invocation
//! under a throwaway `HOME` — tens of seconds each, seventeen tests at once. The
//! stub answers both in microseconds, and pins the version the rc is rendered
//! for rather than leaving it to whatever the host has.

mod common;

use common::aspect_cli;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};

/// Run `aspect setup bazelrc <args>` with `home` as the home directory and a cwd
/// of `cwd`, against an environment that names the endpoints itself.
fn setup_bazelrc(home: &Path, cwd: &Path, args: &[&str]) -> Output {
    with_stub_bazel(&mut Command::new(aspect_cli()))
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

/// A `bazel` that answers without starting one.
///
/// `ctx.bazel.version()` runs `bazel info server_pid release` and the Workflows
/// feature runs `bazel --version`; both are spawned in the test's throwaway cwd
/// with a throwaway `HOME`, so a real binary resolves a release, downloads it
/// and starts a server before either can answer. This prints what those two
/// parse and exits, so the version the rc is rendered for is the same on every
/// machine.
///
/// One directory per test process, kept for the process's life — the harness
/// runs tests as threads, so the first caller builds it and the rest reuse it.
fn stub_bazel_dir() -> &'static Path {
    static DIR: std::sync::OnceLock<PathBuf> = std::sync::OnceLock::new();
    DIR.get_or_init(|| {
        let dir = tempfile::tempdir().expect("temp bin dir").keep();
        let bazel = dir.join("bazel");
        std::fs::write(
            &bazel,
            "#!/bin/sh\n\
             case \"$1\" in\n\
             --version) echo 'bazel 9.0.0' ;;\n\
             *) echo 'server_pid: 1'; echo 'release: release 9.0.0' ;;\n\
             esac\n",
        )
        .expect("writing the bazel stub");
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&bazel, std::fs::Permissions::from_mode(0o755))
                .expect("making the bazel stub executable");
        }
        dir
    })
    .as_path()
}

/// `cmd` with the stub ahead of whatever `bazel` the host has.
fn with_stub_bazel(cmd: &mut Command) -> &mut Command {
    let path = std::env::var("PATH").unwrap_or_default();
    cmd.env("PATH", format!("{}:{}", stub_bazel_dir().display(), path))
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

/// An Aspect Workflows runner takes the home layout without being asked, and is
/// the one base whose execution log sits elsewhere — the job tmpdir, per job
/// rather than per machine.
///
/// The Workflows agent removes `~/.aspect` outright and deletes `~/.bazelrc`
/// before every job, so each job starts from nothing here. What this pins is the
/// footprint that leaves behind for the agent to clear — one file under
/// `~/.aspect`, no generated directory for a log that lands elsewhere — and that
/// a second run over the first's output is a no-op, since the sweep is what the
/// runner relies on rather than something this command needs.
#[test]
fn a_workflows_runner_leaves_only_the_rc_pair_behind() {
    let home = tempfile::tempdir().expect("temp home");
    let cwd = tempfile::tempdir().expect("temp cwd");
    let job_tmpdir = tempfile::tempdir().expect("temp job tmpdir");

    let run = || {
        with_stub_bazel(&mut Command::new(aspect_cli()))
            .args(["setup", "bazelrc"])
            .current_dir(cwd.path())
            .env("HOME", home.path())
            .env(
                "ASPECT_CREDENTIALS_FILE",
                home.path().join("credentials.json"),
            )
            .env("ASPECT_WORKFLOWS_RUNNER", "1")
            .env("ASPECT_WORKFLOWS_RUNNER_JOB_TMPDIR", job_tmpdir.path())
            .env("ASPECT_WORKFLOWS_REMOTE_CACHE", "grpcs://cache.example:443")
            .env("ASPECT_WORKFLOWS_BES_BACKEND", "grpcs://bes.example:443")
            .env_remove("ASPECT_AUTH_PROFILE")
            .env_remove("ASPECT_API_TOKEN")
            .env_remove("ASPECT_DEBUG")
            .output()
            .expect("running `aspect setup bazelrc`")
    };

    assert_success(&run());
    assert_success(&run());

    // `--home` was never passed; the runner marker alone chose the layout.
    let rc = read(&home.path().join(".aspect/bazelrc"));
    assert!(
        rc.contains(&format!(
            "--execution_log_compact_file={}",
            job_tmpdir.path().join("exec.log.zstd").display()
        )),
        "the log belongs to the job, not the home directory:\n{rc}"
    );
    assert!(
        !home.path().join(".aspect/generated").exists(),
        "no directory should be created for a log that lands elsewhere"
    );

    // The whole footprint, however many jobs have run: one file under
    // `~/.aspect`, rewritten, and one import line.
    let under_dot_aspect: Vec<String> = std::fs::read_dir(home.path().join(".aspect"))
        .expect(".aspect")
        .map(|e| e.expect("entry").file_name().to_string_lossy().into_owned())
        .collect();
    assert_eq!(under_dot_aspect, vec!["bazelrc"]);
    assert_eq!(
        read(&home.path().join(".bazelrc"))
            .matches("try-import")
            .count(),
        1
    );
}

/// Anything the machine's owner put in the importing rc is theirs: it is kept
/// below the import, where Bazel's last-wins gives it the final say.
#[test]
fn a_hand_written_home_rc_is_kept_below_the_import() {
    let home = tempfile::tempdir().expect("temp home");
    let cwd = tempfile::tempdir().expect("temp cwd");
    let theirs = "# my own settings\ncommon --disk_cache=/fast/disk\n";
    std::fs::write(home.path().join(".bazelrc"), theirs).expect("seed ~/.bazelrc");

    assert_success(&setup_bazelrc(home.path(), cwd.path(), &["--home"]));

    let rc = read(&home.path().join(".bazelrc"));
    assert!(rc.contains(theirs), "their lines must survive:\n{rc}");
    assert!(
        rc.find("try-import").unwrap() < rc.find("--disk_cache").unwrap(),
        "and must come after the import, so they win:\n{rc}"
    );
}

/// Every variable a CI host is detected by, cleared so a test decides for itself
/// whether it is on CI — these tests themselves run on one.
const CI_VARS: &[&str] = &[
    "CI",
    "GITHUB_ACTIONS",
    "BUILDKITE",
    "BUILDKITE_REPO",
    "BUILDKITE_AGENT_ACCESS_TOKEN",
    "CIRCLECI",
    "GITLAB_CI",
    "GITHUB_REPOSITORY",
    "CI_JOB_TOKEN",
    "CIRCLE_PROJECT_REPONAME",
    "CI_PROJECT_NAME",
];

/// Run without the `ASPECT_WORKFLOWS_*` endpoints, so the rc takes its endpoints
/// from the default deployment rather than from the environment. `ci` decides
/// whether the run looks like CI, which is what `--remote=auto` keys off.
fn setup_bazelrc_unconfigured(home: &Path, cwd: &Path, ci: bool, args: &[&str]) -> Output {
    unconfigured_cmd(home, cwd, ci, args)
        .output()
        .expect("running `aspect setup bazelrc`")
}

/// The same run, unspawned, for a test that adds to its environment — a
/// credential, say, which is what `--remote=auto` looks for.
fn unconfigured_cmd(home: &Path, cwd: &Path, ci: bool, args: &[&str]) -> Command {
    let mut cmd = Command::new(aspect_cli());
    with_stub_bazel(&mut cmd);
    cmd.args(["setup", "bazelrc"])
        .args(args)
        .current_dir(cwd)
        .env("HOME", home)
        .env("ASPECT_CREDENTIALS_FILE", home.join("credentials.json"))
        .env_remove("ASPECT_AUTH_PROFILE")
        .env_remove("ASPECT_API_TOKEN")
        .env_remove("ASPECT_DEBUG")
        .env_remove("ASPECT_WORKFLOWS_RUNNER");
    for var in CI_VARS {
        cmd.env_remove(var);
    }
    if ci {
        cmd.env("CI", "true")
            .env("GITHUB_ACTIONS", "true")
            .env("GITHUB_REPOSITORY", "acme/widgets");
    }
    cmd
}

/// The `common --config=<group>` names the rc turns on for every `bazel` call.
fn enabled_groups(rc: &str) -> Vec<&str> {
    rc.lines()
        .filter_map(|l| l.strip_prefix("common --config="))
        .collect()
}

/// Aspect Cloud is among the sections on a machine that has never logged in,
/// because the built-in entry states its own endpoints.
#[test]
fn a_machine_that_has_never_logged_in_still_gets_an_aspect_cloud_section() {
    let home = tempfile::tempdir().expect("temp home");
    let workspace = tempfile::tempdir().expect("temp workspace");
    std::fs::write(workspace.path().join("MODULE.bazel"), "").expect("MODULE.bazel");

    let output = setup_bazelrc_unconfigured(home.path(), workspace.path(), false, &[]);
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

/// The shape `--home` exists for: a stock CI runner whose environment names no
/// endpoint. `--remote` defaults to `auto`, which on CI is the accelerators that
/// cannot change build semantics, taken from the default deployment — here the
/// built-in Aspect Cloud entry — so a plain `bazel build` is cached and reports
/// without naming a `--config`. Remote execution is never among them.
#[test]
fn on_ci_the_default_deployments_cache_and_bes_are_enabled() {
    let home = tempfile::tempdir().expect("temp home");
    let cwd = tempfile::tempdir().expect("temp cwd");

    let out = unconfigured_cmd(home.path(), cwd.path(), true, &["--home"])
        .env("ASPECT_API_TOKEN", "client:secret")
        .output()
        .expect("running `aspect setup bazelrc`");
    assert_success(&out);

    let rc = read(&home.path().join(".aspect/bazelrc"));
    assert_eq!(
        enabled_groups(&rc),
        vec![
            "aspect-common",
            "aspect-github-actions",
            "aspect-cache",
            "aspect-bes",
            "aspect-exec-log"
        ],
        "in:\n{rc}"
    );
    // Aspect Cloud's endpoints reach the groups, with the credential helper that
    // authenticates them — the environment's own endpoints need no helper, these
    // do.
    assert!(
        rc.contains("common:aspect-cache --remote_cache=grpcs://cache.aspect.build")
            && rc.contains("common:aspect-bes --bes_backend=grpcs://bes.aspect.build")
            && rc.contains("common:aspect-cache --credential_helper=cache.aspect.build=aspect"),
        "in:\n{rc}"
    );
}

/// Bazel treats a credential helper that cannot produce a token as fatal —
/// `Failed to get credentials … from helper` fails the command rather than
/// building locally — so `auto` declines a deployment nothing here can
/// authenticate. Without this a tokenless CI job would get an rc that breaks
/// every `bazel` command, which is worse than the cache it was meant to add.
#[test]
fn auto_declines_a_deployment_with_no_credential() {
    let home = tempfile::tempdir().expect("temp home");
    let cwd = tempfile::tempdir().expect("temp cwd");

    let out = setup_bazelrc_unconfigured(home.path(), cwd.path(), true, &["--home"]);
    assert_success(&out);
    let rc = read(&home.path().join(".aspect/bazelrc"));
    assert_eq!(
        enabled_groups(&rc),
        vec!["aspect-common", "aspect-github-actions"],
        "no endpoint should be enabled with nothing to authenticate it:\n{rc}"
    );
    assert!(
        String::from_utf8_lossy(&out.stderr).contains("nothing here can authenticate it"),
        "the reason must be said out loud"
    );
    // The section stays, so the rc works the moment there is a credential.
    assert!(
        rc.contains("common:aspect-cloud --remote_cache="),
        "in:\n{rc}"
    );

    // An API token is a credential, so the same run enables them.
    let with_token = unconfigured_cmd(home.path(), cwd.path(), true, &["--home", "--force"])
        .env("ASPECT_API_TOKEN", "client:secret")
        .output()
        .expect("running `aspect setup bazelrc`");
    assert_success(&with_token);
    assert!(
        enabled_groups(&read(&home.path().join(".aspect/bazelrc"))).contains(&"aspect-cache"),
        "a token is a credential"
    );

    // An explicit --remote is the caller's call: they may be arranging
    // credentials some other way, so it is honored either way.
    let forced = setup_bazelrc_unconfigured(
        home.path(),
        cwd.path(),
        true,
        &["--home", "--remote", "--force"],
    );
    assert_success(&forced);
    assert!(
        enabled_groups(&read(&home.path().join(".aspect/bazelrc"))).contains(&"aspect-cache"),
        "an explicit --remote is not second-guessed"
    );
}

/// `--remote` is the same grammar as `aspect build --remote`, and selects what
/// the rc turns on. A capability the deployment does not advertise cannot be
/// turned on by asking for it.
#[test]
fn remote_selects_which_endpoints_the_rc_enables() {
    let home = tempfile::tempdir().expect("temp home");
    let cwd = tempfile::tempdir().expect("temp cwd");
    let enabled = |args: &[&str]| {
        let mut argv = vec!["--force"];
        argv.extend_from_slice(args);
        let out = setup_bazelrc_unconfigured(home.path(), cwd.path(), true, &argv);
        assert_success(&out);
        enabled_groups(&read(&home.path().join(".aspect/bazelrc")))
            .iter()
            .map(|s| s.to_string())
            .collect::<Vec<_>>()
    };

    assert_eq!(
        enabled(&["--home", "--remote=none"]),
        ["aspect-common", "aspect-github-actions"]
    );
    assert_eq!(
        enabled(&["--home", "--remote=no-bes"]),
        ["aspect-common", "aspect-github-actions", "aspect-cache"]
    );
    // Aspect Cloud advertises no executor, so naming `exec` adds nothing.
    assert_eq!(
        enabled(&["--home", "--remote=exec"]),
        [
            "aspect-common",
            "aspect-github-actions",
            "aspect-cache",
            "aspect-bes",
            "aspect-exec-log"
        ]
    );

    let bad =
        setup_bazelrc_unconfigured(home.path(), cwd.path(), true, &["--home", "--remote=nope"]);
    assert_eq!(bad.status.code(), Some(1));
    assert!(
        String::from_utf8_lossy(&bad.stderr).contains("unknown capability 'nope'"),
        "expected the shared --remote grammar error"
    );
}

/// An rc that is already there is left as it is, in either base — a checkout's
/// because the repository edits it, a machine's because a persistent runner may
/// have been set up by hand. `--force` is the way back to a generated one, which
/// the file's own first line says.
#[test]
fn an_existing_rc_is_kept_until_force() {
    for base in ["--home", "--home=false"] {
        let home = tempfile::tempdir().expect("temp home");
        let workspace = tempfile::tempdir().expect("temp workspace");
        std::fs::write(workspace.path().join("MODULE.bazel"), "").expect("MODULE.bazel");
        let generated = if base == "--home" {
            home.path().join(".aspect/bazelrc")
        } else {
            workspace.path().join(".aspect/bazelrc")
        };

        assert_success(&setup_bazelrc_unconfigured(
            home.path(),
            workspace.path(),
            true,
            &[base],
        ));
        let edited = read(&generated) + "\ncommon --//my:own\n";
        std::fs::write(&generated, &edited).expect("edit");

        let out = setup_bazelrc_unconfigured(home.path(), workspace.path(), true, &[base]);
        assert_success(&out);
        assert_eq!(
            read(&generated),
            edited,
            "{base}: edits must survive a rerun"
        );

        assert_success(&setup_bazelrc_unconfigured(
            home.path(),
            workspace.path(),
            true,
            &[base, "--force"],
        ));
        assert_ne!(read(&generated), edited, "{base}: --force regenerates");
        assert!(
            read(&generated).contains("--force` regenerates it"),
            "{base}: the file says how to regenerate it"
        );
    }
}

/// Write a checkout that already carries its own generated rc, as committing the
/// output of an off-CI run leaves it.
fn checkout_owning_its_rc(workspace: &Path) {
    std::fs::write(workspace.join("MODULE.bazel"), "").expect("MODULE.bazel");
    std::fs::create_dir_all(workspace.join(".aspect")).expect(".aspect");
    std::fs::write(
        workspace.join(".aspect/bazelrc"),
        "# Generated by `aspect setup bazelrc` as a starting point.\ncommon:aspect-common --heap_dump_on_oom\ncommon:aspect-cloud --remote_cache=grpcs://cache.aspect.build\ncommon:aspect-cloud --remote_timeout=99\n",
    )
    .expect(".aspect/bazelrc");
    std::fs::write(
        workspace.join(".bazelrc"),
        "# Aspect's rc for vanilla `bazel`, maintained by `aspect setup bazelrc`.\ntry-import %workspace%/.aspect/bazelrc\n",
    )
    .expect(".bazelrc");
}

/// A checkout that defines its own sections is *enabled*, not restated: the
/// machine's rc carries only the CI host's group, which a committed rc never
/// holds, and the enable lines. Nothing it writes can override what the
/// repository edited, and CI still gets the cache turned on.
#[test]
fn a_checkout_that_defines_its_own_sections_is_enabled_not_restated() {
    let home = tempfile::tempdir().expect("temp home");
    let workspace = tempfile::tempdir().expect("temp workspace");
    checkout_owning_its_rc(workspace.path());
    let before = read(&workspace.path().join(".aspect/bazelrc"));

    let out = unconfigured_cmd(home.path(), workspace.path(), true, &[])
        .env("ASPECT_API_TOKEN", "client:secret")
        .output()
        .expect("running `aspect setup bazelrc`");
    assert_success(&out);

    let rc = read(&home.path().join(".aspect/bazelrc"));
    assert_eq!(
        enabled_groups(&rc),
        vec!["aspect-github-actions", "aspect-common", "aspect-cloud"],
        "in:\n{rc}"
    );
    // Nothing restated, so nothing of the repository's can be overridden.
    assert!(
        !rc.contains("common:aspect-common ") && !rc.contains("common:aspect-cloud "),
        "the checkout's own sections must not be redefined:\n{rc}"
    );
    assert_eq!(
        before,
        read(&workspace.path().join(".aspect/bazelrc")),
        "and the repository's file is untouched"
    );
}

/// The section looked for is the *default deployment's*, not `aspect-cloud`
/// literally. A repo rc generated by someone logged in only to Aspect Cloud has
/// no `aspect-silo-aws`, so CI whose default is that deployment falls back to
/// carrying its own definitions — the likelier direction in practice.
#[test]
fn a_configured_default_looks_for_its_own_section() {
    let home = tempfile::tempdir().expect("temp home");
    let workspace = tempfile::tempdir().expect("temp workspace");
    checkout_owning_its_rc(workspace.path());

    std::fs::create_dir_all(home.path().join(".aspect")).expect(".aspect");
    std::fs::write(
        home.path().join(".aspect/config.json"),
        concat!(
            r#"{"deployments":[{"name":"silo-aws","default":true,"#,
            r#""issuer":"https://auth.aspect.build","endpoints":{"#,
            r#""cache":"remote.silo-aws.aspect.build","#,
            r#""bes":"remote.silo-aws.aspect.build","#,
            r#""results_url":"https://app.silo-aws.aspect.build/i/"}}]}"#,
        ),
    )
    .expect("config.json");

    let out = unconfigured_cmd(home.path(), workspace.path(), true, &[])
        .env("ASPECT_API_TOKEN_SILO_AWS", "client:secret")
        .output()
        .expect("running `aspect setup bazelrc`");
    assert_success(&out);
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(
        stderr.contains("does not define aspect-silo-aws"),
        "the missing section is named by the default deployment:\n{stderr}"
    );

    let rc = read(&home.path().join(".aspect/bazelrc"));
    assert!(
        rc.contains("common:aspect-cache --remote_cache=grpcs://remote.silo-aws.aspect.build"),
        "our own definitions, from the default deployment:\n{rc}"
    );
}

/// Bazel fails outright on a `--config` no rc defines, so enabling the
/// checkout's sections is founded on reading which ones it has. A repo whose rc
/// was generated against a different deployment has no `aspect-cloud`, and must
/// get our own definitions rather than a broken enable line.
#[test]
fn a_checkout_missing_the_section_gets_our_own_definitions() {
    let home = tempfile::tempdir().expect("temp home");
    let workspace = tempfile::tempdir().expect("temp workspace");
    checkout_owning_its_rc(workspace.path());
    // Regenerated by someone logged in only to a single-tenant deployment.
    std::fs::write(
        workspace.path().join(".aspect/bazelrc"),
        "# Generated by `aspect setup bazelrc` as a starting point.\ncommon:aspect-common --heap_dump_on_oom\ncommon:aspect-silo-aws --remote_cache=grpcs://x\n",
    )
    .expect(".aspect/bazelrc");

    let out = unconfigured_cmd(home.path(), workspace.path(), true, &[])
        .env("ASPECT_API_TOKEN", "client:secret")
        .output()
        .expect("running `aspect setup bazelrc`");
    assert_success(&out);
    assert!(
        String::from_utf8_lossy(&out.stderr).contains("does not define aspect-cloud"),
        "the shortfall must be said out loud"
    );

    let rc = read(&home.path().join(".aspect/bazelrc"));
    assert!(
        enabled_groups(&rc).contains(&"aspect-cache")
            && rc.contains("common:aspect-cache --remote_cache="),
        "our own endpoint definitions, not a dangling --config:\n{rc}"
    );
    assert!(
        !rc.contains("--config=aspect-cloud\n"),
        "no enable line for a section that is not there:\n{rc}"
    );
}

/// The exemption: a Workflows runner's endpoints and paths are its own, which no
/// checkout can describe and which are meant to win, so its rc is written even
/// where the repository has one. Every Workflows customer's repo is in exactly
/// this shape.
#[test]
fn a_workflows_runner_writes_its_rc_even_when_the_checkout_owns_one() {
    let home = tempfile::tempdir().expect("temp home");
    let workspace = tempfile::tempdir().expect("temp workspace");
    let job_tmpdir = tempfile::tempdir().expect("temp job tmpdir");
    checkout_owning_its_rc(workspace.path());

    let out = with_stub_bazel(&mut Command::new(aspect_cli()))
        .args(["setup", "bazelrc"])
        .current_dir(workspace.path())
        .env("HOME", home.path())
        .env(
            "ASPECT_CREDENTIALS_FILE",
            home.path().join("credentials.json"),
        )
        .env("ASPECT_WORKFLOWS_RUNNER", "1")
        .env("ASPECT_WORKFLOWS_RUNNER_JOB_TMPDIR", job_tmpdir.path())
        .env("ASPECT_WORKFLOWS_REMOTE_CACHE", "grpcs://cache.example:443")
        .env_remove("ASPECT_AUTH_PROFILE")
        .env_remove("ASPECT_API_TOKEN")
        .env_remove("ASPECT_DEBUG")
        .output()
        .expect("running `aspect setup bazelrc`");
    assert_success(&out);

    let rc = read(&home.path().join(".aspect/bazelrc"));
    assert!(
        rc.contains("--remote_cache=grpcs://cache.example:443"),
        "in:\n{rc}"
    );
}

/// CI selects the home layout on its own: a CI checkout is a throwaway that some
/// jobs commit from, so nothing of this machine's belongs in it. The integrations
/// pass `--home` explicitly anyway, and the two must agree.
#[test]
fn ci_chooses_the_home_layout_without_being_asked() {
    let auto = tempfile::tempdir().expect("temp home");
    let explicit = tempfile::tempdir().expect("temp home");
    let workspace = tempfile::tempdir().expect("temp workspace");
    std::fs::write(workspace.path().join("MODULE.bazel"), "").expect("MODULE.bazel");

    assert_success(&setup_bazelrc_unconfigured(
        auto.path(),
        workspace.path(),
        true,
        &[],
    ));
    assert_success(&setup_bazelrc_unconfigured(
        explicit.path(),
        workspace.path(),
        true,
        &["--home"],
    ));

    // A workspace was right there and neither run touched it.
    assert!(auto.path().join(".aspect/bazelrc").is_file());
    assert!(!workspace.path().join(".aspect").exists());
    assert_eq!(
        read(&auto.path().join(".aspect/bazelrc"))
            .replace(&auto.path().to_string_lossy().to_string(), "<home>"),
        read(&explicit.path().join(".aspect/bazelrc"))
            .replace(&explicit.path().to_string_lossy().to_string(), "<home>"),
        "--home and the CI default must produce the same rc"
    );
}

/// A committed rc is shared by every host that builds the repository, so the
/// default deployment — this machine's setting — never reaches it. `--home=false`
/// is how a job whose purpose is regenerating that file opts back into it.
#[test]
fn a_checkout_rc_enables_nothing_even_on_ci() {
    let home = tempfile::tempdir().expect("temp home");
    let workspace = tempfile::tempdir().expect("temp workspace");
    std::fs::write(workspace.path().join("MODULE.bazel"), "").expect("MODULE.bazel");

    assert_success(&setup_bazelrc_unconfigured(
        home.path(),
        workspace.path(),
        true,
        &["--home=false"],
    ));

    let rc = read(&workspace.path().join(".aspect/bazelrc"));
    assert_eq!(enabled_groups(&rc), Vec::<&str>::new(), "in:\n{rc}");
    assert!(
        !rc.contains("aspect-github-actions"),
        "the CI host's group is this machine's, not the repository's:\n{rc}"
    );
    // The endpoints stay where a checkout can only reach them by name.
    assert!(
        !rc.contains("common:aspect-cache --remote_cache=")
            && rc.contains("common:aspect-cloud --remote_cache="),
        "in:\n{rc}"
    );
}

/// Off CI `auto` turns nothing on: a developer's rc stays inert until a
/// `--config` names something, so it cannot silently redirect a local build.
#[test]
fn home_mode_enables_the_endpointless_tuning_and_says_the_files_are_this_machines() {
    let home = tempfile::tempdir().expect("temp home");
    let cwd = tempfile::tempdir().expect("temp cwd");

    let output = setup_bazelrc_unconfigured(home.path(), cwd.path(), false, &["--home"]);
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
