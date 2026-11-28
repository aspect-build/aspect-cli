//! `aspect axl add` refuses a dependency name `MODULE.aspect` would refuse,
//! whether it comes from the repository or from `--name`: one the CLI gives a
//! module of its own (`root`, `aspect`), or one that is no module name
//! (`1password_cli`). Writing it would fail every command. The refusal comes
//! before anything is fetched, so these runs need no network.

mod common;

use common::{aspect_cli, base_env};
use std::process::Command;

#[test]
fn a_name_module_aspect_would_refuse_is_refused_before_anything_is_fetched() {
    for (args, reason) in [
        (&["gh:someone/aspect"][..], "provides itself"),
        (
            &["gh:someone/thing", "--name=aspect"][..],
            "provides itself",
        ),
        (&["gh:someone/thing", "--name=root"][..], "provides itself"),
        (
            &["gh:someone/1password-cli"][..],
            "begin with a lowercase letter",
        ),
        (
            &["gh:someone/thing-"][..],
            "end with a lowercase letter or digit",
        ),
        (
            &["gh:someone/thing", "--name=My.Dep"][..],
            "begin with a lowercase letter",
        ),
    ] {
        let dir = tempfile::tempdir().expect("temp dir");
        std::fs::write(dir.path().join("MODULE.aspect"), "").unwrap();
        let out = Command::new(aspect_cli())
            .args(["axl", "add"])
            .args(args)
            .current_dir(dir.path())
            .env_clear()
            .envs(base_env())
            .env("HOME", dir.path().join("home"))
            .env("ASPECT_CLI_CACHE", dir.path().join("cache"))
            .output()
            .expect("running the CLI");
        let stderr = String::from_utf8_lossy(&out.stderr);
        assert_eq!(out.status.code(), Some(1), "axl add {args:?}:\n{stderr}");
        assert!(
            stderr.contains("Cannot name the dependency")
                && stderr.contains(reason)
                && stderr.contains("--name=<name>"),
            "axl add {args:?}: expected {reason:?} in:\n{stderr}"
        );
        assert!(
            !stderr.contains("Querying GitHub"),
            "axl add {args:?} fetched:\n{stderr}"
        );
        assert_eq!(
            std::fs::read_to_string(dir.path().join("MODULE.aspect")).unwrap(),
            "",
            "axl add {args:?} wrote MODULE.aspect"
        );
    }
}
