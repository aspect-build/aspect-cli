//! The WorktreePool feature's options reach the worktree tasks: an out-of-range
//! `max_slots`, `abandoned_grace_hours` or `max_idle_days`, set on the command
//! line or in `config.axl`, is refused by `aspect worktree add` naming the
//! option, and the defaults (or a disabled feature) pass the check.
//!
//! For those checks the scratch workspace is not a git repository, so `add`
//! checks the settings and then refuses with `not_a_repository`: no worktree
//! is ever created. The idle limit's effect is checked in a scratch clone with
//! its own `HOME`, no system git config and no ambient `GIT_*` variables, so
//! the pool that `add` sweeps is the test's own.

mod common;

use common::{aspect_cli, base_env};
use std::path::{Path, PathBuf};
use std::process::{Command, Output};

/// Run `aspect worktree add probe --output=json <args>` in a scratch workspace
/// under an empty `$HOME`, with `.aspect/config.axl` holding `config`, if given.
fn add(args: &[&str], config: Option<&str>) -> Output {
    let dir = tempfile::tempdir().expect("temp dir");
    std::fs::write(dir.path().join("MODULE.aspect"), "").expect("MODULE.aspect");
    if let Some(config) = config {
        let aspect = dir.path().join(".aspect");
        std::fs::create_dir(&aspect).expect(".aspect");
        std::fs::write(aspect.join("config.axl"), config).expect("config.axl");
    }
    let output = Command::new(aspect_cli())
        .args(["worktree", "add", "probe", "--output=json"])
        .args(args)
        .current_dir(dir.path())
        .env_clear()
        .envs(base_env())
        .env("ASPECT_CLI_CACHE", dir.path().join("cache"))
        .env("HOME", dir.path().join("home"))
        // Keep git from finding a repository above the scratch dir.
        .env(
            "GIT_CEILING_DIRECTORIES",
            dir.path().parent().expect("temp dir parent"),
        )
        .output()
        .unwrap_or_else(|e| panic!("running `aspect worktree add {}`: {e}", args.join(" ")));
    dir.close().expect("removing temp dir");
    output
}

/// The `error` code of `out`'s JSON refusal on stdout.
fn error(out: &Output) -> String {
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert_eq!(
        out.status.code(),
        Some(1),
        "{stdout}\n{}",
        String::from_utf8_lossy(&out.stderr)
    );
    stdout
        .lines()
        .find_map(|l| l.trim().strip_prefix("\"error\": \""))
        .and_then(|rest| rest.split('"').next())
        .unwrap_or_else(|| panic!("no error code in:\n{stdout}"))
        .to_owned()
}

/// `out` refused the setting `option` as `invalid_config`.
fn assert_refused(out: Output, option: &str) {
    assert_eq!(error(&out), "invalid_config");
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(
        stdout.contains(&format!("\"setting\": \"{option}\"")),
        "{stdout}"
    );
}

#[test]
fn defaults_pass_the_check() {
    assert_eq!(error(&add(&[], None)), "not_a_repository");
}

#[test]
fn an_out_of_range_flag_is_refused_naming_it() {
    assert_refused(
        add(&["--worktree-pool:max-slots=0"], None),
        "--worktree-pool:max-slots",
    );
    assert_refused(
        add(&["--worktree-pool:abandoned-grace-hours=-1"], None),
        "--worktree-pool:abandoned-grace-hours",
    );
    assert_refused(
        add(&["--worktree-pool:max-idle-days=0"], None),
        "--worktree-pool:max-idle-days",
    );
}

#[test]
fn a_config_axl_value_reaches_the_tasks() {
    let config = r#"load("@aspect//feature/worktree_pool.axl", "WorktreePool")

def config(ctx):
    ctx.features[WorktreePool].args.abandoned_grace_hours = -1
"#;
    assert_refused(
        add(&[], Some(config)),
        "--worktree-pool:abandoned-grace-hours",
    );
    let config = r#"load("@aspect//feature/worktree_pool.axl", "WorktreePool")

def config(ctx):
    ctx.features[WorktreePool].args.max_idle_days = -1
"#;
    assert_refused(add(&[], Some(config)), "--worktree-pool:max-idle-days");
}

/// `cmd` run in the clone at `dir/repo`, with `dir/home` as `$HOME`, no system
/// git config and none of the caller's `GIT_*` variables.
fn in_clone(mut cmd: Command, dir: &Path) -> Command {
    for (k, _) in std::env::vars_os() {
        if k.to_string_lossy().starts_with("GIT_") {
            cmd.env_remove(k);
        }
    }
    cmd.current_dir(dir.join("repo"))
        .env("HOME", dir.join("home"))
        .env("XDG_CONFIG_HOME", dir.join("home/.config"))
        .env("GIT_CONFIG_NOSYSTEM", "1");
    cmd
}

fn git(dir: &Path, args: &[&str]) {
    let status = in_clone(Command::new("git"), dir)
        .args(["-c", "user.name=t", "-c", "user.email=t@example.com"])
        .args(["-c", "commit.gpgsign=false"])
        .args(args)
        .status()
        .expect("running git");
    assert!(status.success(), "git {args:?} failed");
}

/// `aspect worktree <args>` in the clone at `dir/repo`; asserts it succeeded.
fn worktree(dir: &Path, args: &[&str]) -> Output {
    let mut cmd = Command::new(aspect_cli());
    cmd.env_clear().envs(base_env());
    let out = in_clone(cmd, dir)
        .env("ASPECT_CLI_CACHE", dir.join("cache"))
        .arg("worktree")
        .args(args)
        .output()
        .expect("running the CLI");
    assert!(
        out.status.success(),
        "aspect worktree {args:?} failed:\n{}",
        String::from_utf8_lossy(&out.stderr)
    );
    out
}

/// The one pool registry under `dir`, found without walking into the slots.
fn registry(dir: &Path) -> PathBuf {
    let mut found = Vec::new();
    let mut todo = vec![dir.join("home/.aspect/worktrees")];
    while let Some(d) = todo.pop() {
        for entry in std::fs::read_dir(&d).expect("reading the pools").flatten() {
            let name = entry.file_name().to_string_lossy().into_owned();
            if name == "registry.json" {
                found.push(entry.path());
            } else if entry.file_type().is_ok_and(|t| t.is_dir())
                && !name.starts_with("aspect-worktree-")
            {
                todo.push(entry.path());
            }
        }
    }
    assert_eq!(found.len(), 1, "registries: {found:?}");
    found.remove(0)
}

/// Make every slot of the pool under `dir` read as last used `days` ago: its
/// creation and every lease taken and given back.
fn age_slots(dir: &Path, days: u64) {
    let path = registry(dir);
    let mut doc: serde_json::Value =
        serde_json::from_slice(&std::fs::read(&path).expect("reading the registry"))
            .expect("registry JSON");
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .expect("clock")
        .as_millis() as u64;
    let then = serde_json::json!(now - days * 24 * 3600 * 1000);
    for slot in doc["slots"].as_object_mut().expect("slots").values_mut() {
        slot["created_ms"] = then.clone();
        for entry in slot["history"].as_array_mut().into_iter().flatten() {
            for key in ["leased_ms", "released_ms"] {
                if entry.get(key).is_some_and(|v| v.as_u64() != Some(0)) {
                    entry[key] = then.clone();
                }
            }
        }
    }
    std::fs::write(&path, serde_json::to_vec(&doc).expect("encoding"))
        .expect("writing the registry");
}

/// A scratch clone with `config` as its `config.axl`, whose pool holds one
/// free slot last used 45 days ago, and that slot's directory.
fn clone_with_a_45_day_idle_slot(config: &str) -> (tempfile::TempDir, String) {
    let dir = tempfile::tempdir().expect("temp dir");
    std::fs::create_dir_all(dir.path().join("repo/.aspect")).unwrap();
    std::fs::create_dir_all(dir.path().join("home")).unwrap();
    std::fs::write(dir.path().join("repo/MODULE.aspect"), "").unwrap();
    std::fs::write(dir.path().join("repo/.aspect/config.axl"), config).unwrap();
    git(dir.path(), &["init", "-q"]);
    git(dir.path(), &["commit", "-q", "--allow-empty", "-m", "init"]);
    let out = worktree(
        dir.path(),
        &["add", "first", "--create=HEAD", "--output=path"],
    );
    let slot = String::from_utf8_lossy(&out.stdout).trim().to_owned();
    worktree(dir.path(), &["release", "first"]);
    age_slots(dir.path(), 45);
    (dir, slot)
}

/// The directory `add` gives a new branch in the clone at `dir`, and what it
/// printed on stderr.
fn add_second(dir: &Path) -> (String, String) {
    let out = worktree(dir, &["add", "second", "--create=HEAD", "--output=path"]);
    let said = String::from_utf8_lossy(&out.stderr).into_owned();
    (String::from_utf8_lossy(&out.stdout).trim().to_owned(), said)
}

/// `prune --dry-run --output=json <args>` in the clone at `dir`: the limit it
/// reports, and the directories of the slots it would delete.
fn prune_preview(dir: &Path, args: &[&str]) -> (i64, Vec<String>) {
    let out = worktree(
        dir,
        &[&["prune", "--dry-run", "--output=json"], args].concat(),
    );
    let doc: serde_json::Value = serde_json::from_slice(&out.stdout).expect("prune JSON");
    let paths = doc["candidates"]
        .as_array()
        .expect("candidates")
        .iter()
        .map(|c| c["path"].as_str().expect("path").to_owned())
        .collect();
    (doc["max_idle_days"].as_i64().expect("max_idle_days"), paths)
}

#[test]
fn a_configured_idle_limit_reaches_add_and_prune() {
    let (dir, slot) = clone_with_a_45_day_idle_slot("def config(ctx):\n    pass\n");
    let (second, said) = add_second(dir.path());
    assert!(said.contains("pruned slot (idle "), "{said}");
    assert_ne!(
        slot, second,
        "the default limit kept a 45-day slot:\n{said}"
    );
    dir.close().expect("removing temp dir");

    let config = r#"load("@aspect//feature/worktree_pool.axl", "WorktreePool")

def config(ctx):
    ctx.features[WorktreePool].args.max_idle_days = 90
"#;
    let (dir, slot) = clone_with_a_45_day_idle_slot(config);
    assert_eq!(prune_preview(dir.path(), &[]), (90, vec![]));
    assert_eq!(
        prune_preview(dir.path(), &["--worktree-pool:max-idle-days=0"]),
        (0, vec![slot.clone()]),
    );
    let (second, said) = add_second(dir.path());
    assert!(!said.contains("pruned slot"), "{said}");
    assert_eq!(
        slot, second,
        "a 90-day limit did not keep the 45-day slot:\n{said}"
    );
    dir.close().expect("removing temp dir");
}

#[test]
fn a_disabled_feature_leaves_the_defaults() {
    let config = r#"load("@aspect//feature/worktree_pool.axl", "WorktreePool")

def config(ctx):
    ctx.features[WorktreePool].enabled = False
    ctx.features[WorktreePool].args.max_slots = 0
"#;
    assert_eq!(error(&add(&[], Some(config))), "not_a_repository");
}
