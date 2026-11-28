//! `aspect axl add` refuses to name a dependency after a module the CLI
//! provides (`root`, `aspect`), whether the name comes from the repository or
//! from `--name`: `MODULE.aspect` refuses such a dependency, so writing one
//! would fail every command. The refusal comes before anything is fetched, so
//! these runs need no network.

mod common;

use common::{aspect_cli, base_env};
use std::process::Command;

#[test]
fn a_reserved_module_name_is_refused_before_anything_is_fetched() {
    for args in [
        &["gh:someone/aspect"][..],
        &["gh:someone/thing", "--name=aspect"][..],
        &["gh:someone/thing", "--name=root"][..],
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
            stderr.contains("is the name of a module the CLI provides itself")
                && stderr.contains("--name=<name>"),
            "axl add {args:?}:\n{stderr}"
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
