//! `aspect tips silence|unsilence|list` against a real CLI: where a silence
//! comes from (the user's `~/.aspect/config.json`, the repository's
//! `.aspect/config.json`, a `config.axl`, `--tips:silence`), and that
//! concurrent `silence` processes lose no id.
//!
//! Each test builds its own repository and `$HOME` in a temp folder, so the
//! developer's real home is never read or written.

mod common;

use common::{aspect_cli, base_env};
use std::path::Path;
use std::process::{Command, Output};

const CONFIG_AXL: &str = r#"
load("@aspect//feature/tips.axl", "Tips")

def config(ctx):
    ctx.features[Tips].args.silence = ["from-config"]
"#;

/// A repository in `dir` with `config.axl` (none when empty) and, when given, a
/// `.aspect/config.json`; `$HOME` is `dir/home`.
fn repo(dir: &Path, config_axl: &str, repo_json: Option<&str>) {
    std::fs::write(dir.join("MODULE.aspect"), "").unwrap();
    let aspect = dir.join(".aspect");
    std::fs::create_dir_all(&aspect).unwrap();
    std::fs::create_dir_all(dir.join("home")).unwrap();
    if !config_axl.is_empty() {
        std::fs::write(aspect.join("config.axl"), config_axl).unwrap();
    }
    if let Some(json) = repo_json {
        std::fs::write(aspect.join("config.json"), json).unwrap();
    }
}

fn run(dir: &Path, cache: &str, args: &[&str]) -> Output {
    Command::new(aspect_cli())
        .args(args)
        .current_dir(dir)
        .env_clear()
        .envs(base_env())
        .env("ASPECT_CLI_CACHE", dir.join(cache))
        .env("HOME", dir.join("home"))
        .output()
        .unwrap_or_else(|e| panic!("running `aspect {}`: {e}", args.join(" ")))
}

fn ok(output: &Output, what: &str) -> String {
    let stderr = String::from_utf8_lossy(&output.stderr).into_owned();
    assert_eq!(output.status.code(), Some(0), "{what}:\n{stderr}");
    stderr
}

/// `aspect tips list --output=json` as `{id: [sources]}`.
fn listed(dir: &Path, extra: &[&str]) -> serde_json::Value {
    let mut args = vec!["tips", "list", "--output=json"];
    args.extend_from_slice(extra);
    let output = run(dir, "cache", &args);
    ok(&output, "tips list");
    let doc: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    let mut out = serde_json::Map::new();
    for entry in doc["silenced"].as_array().unwrap() {
        out.insert(
            entry["id"].as_str().unwrap().to_owned(),
            entry["sources"].clone(),
        );
    }
    out.into()
}

#[test]
fn list_names_every_source_and_the_commands_write_only_the_users_file() {
    let dir = tempfile::tempdir().unwrap();
    let repo_json = r#"{"tips": {"silence": ["from-repo", "shared"]}}"#;
    repo(dir.path(), CONFIG_AXL, Some(repo_json));

    let output = run(dir.path(), "cache", &["tips", "silence", "mine", "shared"]);
    let err = ok(&output, "tips silence");
    assert!(err.contains("aspect tips unsilence mine"), "{err}");
    let user = dir.path().join("home/.aspect/config.json");
    let written: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(&user).unwrap()).unwrap();
    assert_eq!(
        written,
        serde_json::json!({"tips": {"silence": ["mine", "shared"]}})
    );

    assert_eq!(
        listed(dir.path(), &[]),
        serde_json::json!({
            "from-config": ["config"],
            "mine": ["user"],
            "shared": ["user", "repo"],
            "from-repo": ["repo"],
        })
    );
    // The command line's ids join config.axl's rather than replacing them.
    let with_flag = listed(
        dir.path(),
        &["--tips:silence=typed", "--tips:silence=from-config"],
    );
    assert_eq!(
        with_flag,
        serde_json::json!({
            "typed": ["flag"],
            "from-config": ["flag", "config"],
            "mine": ["user"],
            "shared": ["user", "repo"],
            "from-repo": ["repo"],
        })
    );

    // One bad id writes none of them.
    let output = run(dir.path(), "cache", &["tips", "silence", "a", "b c", "d"]);
    assert_eq!(output.status.code(), Some(1));
    let after: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(&user).unwrap()).unwrap();
    assert_eq!(after, written, "nothing written");

    let output = run(dir.path(), "cache", &["tips", "unsilence", "shared"]);
    let err = ok(&output, "tips unsilence");
    assert!(err.contains("still silenced by the repository's"), "{err}");
    assert_eq!(
        listed(dir.path(), &[])["shared"],
        serde_json::json!(["repo"])
    );
    assert_eq!(
        std::fs::read_to_string(dir.path().join(".aspect/config.json")).unwrap(),
        repo_json,
        "the repository's file is never written"
    );
}

#[test]
fn an_invalid_user_value_warns_and_is_ignored() {
    let dir = tempfile::tempdir().unwrap();
    repo(
        dir.path(),
        "",
        Some(r#"{"tips": {"silence": ["from-repo"]}}"#),
    );
    std::fs::create_dir_all(dir.path().join("home/.aspect")).unwrap();
    std::fs::write(
        dir.path().join("home/.aspect/config.json"),
        r#"{"tips": {"silence": "nope"}}"#,
    )
    .unwrap();
    let output = run(dir.path(), "cache", &["tips", "list"]);
    let err = ok(&output, "tips list");
    let warnings: Vec<&str> = err
        .lines()
        .filter(|l| l.contains("WARNING") && l.contains("\"tips\""))
        .collect();
    assert_eq!(warnings.len(), 1, "warned once:\n{err}");
    assert!(warnings[0].contains("home/.aspect/config.json"), "{err}");
    assert!(err.contains("from-repo  repo"), "the rest kept:\n{err}");
}

/// `--tips:silence=a,b` is one id, which no tip has: a warning, once
/// however often it is given, says to repeat the flag, and nothing is
/// silenced. A `config.json` entry like it is dropped alone, the file's
/// other ids kept.
#[test]
fn a_comma_id_warns_once_and_silences_nothing() {
    let dir = tempfile::tempdir().unwrap();
    repo(
        dir.path(),
        "",
        Some(r#"{"tips": {"silence": ["kept", "x,y"]}}"#),
    );
    let args = ["tips", "list", "--tips:silence=a,b", "--tips:silence=a,b"];
    let output = run(dir.path(), "cache", &args);
    let err = ok(&output, "tips list");
    let flag: Vec<&str> = err
        .lines()
        .filter(|l| l.contains("--tips:silence \"a,b\" is not a tip id"))
        .collect();
    assert_eq!(flag.len(), 1, "warned once:\n{err}");
    assert!(flag[0].contains("repeat the flag"), "{err}");
    let repo_line = err
        .lines()
        .find(|l| l.contains("\"x,y\" is not a tip id"))
        .unwrap_or_else(|| panic!("no repo warning:\n{err}"));
    assert!(repo_line.contains(".aspect/config.json"), "{repo_line}");
    assert!(
        repo_line.contains("split it into list items"),
        "{repo_line}"
    );
    assert!(
        err.contains("kept  repo"),
        "the file's valid id counts:\n{err}"
    );
}

/// A file that is not JSON is one warning, though both its `editor` and its
/// `tips` are read.
#[test]
fn a_broken_file_warns_once() {
    let dir = tempfile::tempdir().unwrap();
    repo(dir.path(), "", None);
    std::fs::create_dir_all(dir.path().join("home/.aspect")).unwrap();
    std::fs::write(dir.path().join("home/.aspect/config.json"), "{").unwrap();
    let output = run(dir.path(), "cache", &["tips", "list"]);
    let err = ok(&output, "tips list");
    let warnings: Vec<&str> = err
        .lines()
        .filter(|l| l.contains("WARNING") && l.contains("home/.aspect/config.json"))
        .collect();
    assert_eq!(warnings.len(), 1, "warned once:\n{err}");
    assert!(warnings[0].contains("failed to parse"), "{err}");
}

/// Processes, not threads: each CLI takes the lock and writes on its own.
#[test]
fn concurrent_silence_processes_lose_no_id() {
    let dir = tempfile::tempdir().unwrap();
    repo(dir.path(), "", None);
    let ids: Vec<String> = (0..8).map(|i| format!("tip-{i}")).collect();
    std::thread::scope(|s| {
        for (i, id) in ids.iter().enumerate() {
            let dir = dir.path();
            s.spawn(move || {
                let cache = format!("cache-{i}");
                ok(&run(dir, &cache, &["tips", "silence", id]), id);
            });
        }
    });
    let user = dir.path().join("home/.aspect/config.json");
    let written: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(&user).unwrap()).unwrap();
    let mut got: Vec<String> = written["tips"]["silence"]
        .as_array()
        .unwrap()
        .iter()
        .map(|v| v.as_str().unwrap().to_owned())
        .collect();
    got.sort();
    assert_eq!(got, ids);
}
