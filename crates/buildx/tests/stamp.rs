//! Real builds (ignored by default, they need a Bazel 9.0.1 release in the
//! buildx cache) through Bazel's own workspace status action: stamped targets,
//! stable and volatile keys, the `workspace_status` event and the metrics.

use axl_proto::build_event_stream::{BuildEvent, build_event};
use buildx::bep;

struct Workspace {
    dir: std::path::PathBuf,
}

struct Run {
    success: bool,
    stdout: String,
    stderr: String,
    events: Vec<BuildEvent>,
}

impl Run {
    fn total(&self) -> i32 {
        self.runner("total")
    }

    fn internal(&self) -> i32 {
        self.runner("internal")
    }

    fn runner(&self, name: &str) -> i32 {
        self.events
            .iter()
            .find_map(|e| match &e.payload {
                Some(build_event::Payload::BuildMetrics(m)) => m
                    .action_summary
                    .as_ref()
                    .map(|a| a.runner_count.iter().find(|r| r.name == name)),
                _ => None,
            })
            .flatten()
            .map_or(0, |r| r.count)
    }

    /// The keys and values of the `workspace_status` event.
    fn status(&self) -> Vec<(String, String)> {
        self.events
            .iter()
            .find_map(|e| match &e.payload {
                Some(build_event::Payload::WorkspaceStatus(s)) => Some(
                    s.item
                        .iter()
                        .map(|i| (i.key.clone(), i.value.clone()))
                        .collect(),
                ),
                _ => None,
            })
            .unwrap_or_default()
    }

    fn value(&self, key: &str) -> Option<String> {
        self.status()
            .into_iter()
            .find(|(k, _)| k == key)
            .map(|(_, v)| v)
    }
}

impl Workspace {
    fn new(name: &str) -> Option<Workspace> {
        let cached = dirs_cache()?.join("buildx/bazel/9.0.1/A-server.jar");
        if !cached.exists() {
            eprintln!("skipped: no Bazel 9.0.1 release in the buildx cache");
            return None;
        }
        let dir = std::env::temp_dir().join(format!("buildx-stamp-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join("tools")).unwrap();
        let write = |p: &str, text: &str| std::fs::write(dir.join(p), text).unwrap();
        write("MODULE.bazel", "module(name = \"fixture\")\n");
        write(".bazelversion", "9.0.1\n");
        write(
            "BUILD.bazel",
            r#"genrule(
    name = "stamped",
    outs = ["stamped.txt"],
    cmd = "cat bazel-out/stable-status.txt bazel-out/volatile-status.txt | grep -E 'STABLE_GIT_COMMIT|BUILD_SCM_STATUS' > $@",
    stamp = 1,
)
genrule(name = "plain", outs = ["plain.txt"], cmd = "echo plain > $@")
"#,
        );
        write(
            "tools/status.sh",
            "#!/bin/sh\n. ./status.env\necho \"STABLE_GIT_COMMIT $COMMIT\"\necho \"BUILD_SCM_STATUS $SCM\"\n",
        );
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(
            dir.join("tools/status.sh"),
            std::fs::Permissions::from_mode(0o755),
        )
        .unwrap();
        let ws = Workspace { dir };
        ws.status_env("abc123", "clean");
        Some(ws)
    }

    fn status_env(&self, commit: &str, scm: &str) {
        std::fs::write(
            self.dir.join("status.env"),
            format!("COMMIT={commit}\nSCM={scm}\n"),
        )
        .unwrap();
    }

    /// Builds `//:stamped` and `//:plain` with `settings` (Starlark keyword
    /// arguments of `ctx.config`) and reads the event file back.
    fn build(&self, settings: &str) -> Run {
        self.build_targets(settings, "\"//:stamped\", \"//:plain\"")
    }

    fn build_targets(&self, settings: &str, targets: &str) -> Run {
        let script = format!(
            "def _i(ctx: Context):\n    cfg = ctx.config({settings})\n    r = ctx.build([{targets}], config = cfg, events = [ctx.events.file(\"out.bep\")])\n    for label, res in r.items():\n        for a in res.artifacts():\n            ctx.output.print(label, a.digest)\nmain = buildx_main(impl = _i, cli_args = {{}})\n"
        );
        std::fs::write(self.dir.join("build.star"), script).unwrap();
        let _ = std::fs::remove_file(self.dir.join("out.bep"));
        let out = std::process::Command::new(env!("CARGO_BIN_EXE_buildx"))
            .arg("build.star:main")
            .current_dir(&self.dir)
            .env("XDG_CACHE_HOME", self.dir.join("cache"))
            .output()
            .unwrap();
        let events = std::fs::read(self.dir.join("out.bep"))
            .ok()
            .and_then(|b| bep::read_events(&b).ok())
            .unwrap_or_default();
        Run {
            success: out.status.success(),
            stdout: String::from_utf8_lossy(&out.stdout).into_owned(),
            stderr: String::from_utf8_lossy(&out.stderr).into_owned(),
            events,
        }
    }
}

impl Drop for Workspace {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.dir);
    }
}

fn dirs_cache() -> Option<std::path::PathBuf> {
    let home = std::path::PathBuf::from(std::env::var_os("HOME")?);
    Some(if cfg!(target_os = "macos") {
        home.join("Library/Caches")
    } else {
        home.join(".cache")
    })
}

const STAMPED: &str =
    "stamp = True, workspace_status_command = \"tools/status.sh\", embed_label = \"r1\"";

#[test]
#[ignore = "builds with a real Bazel release: cargo test -p buildx --test stamp -- --ignored"]
fn stable_keys_rebuild_stamped_targets_and_volatile_keys_do_not() {
    let Some(ws) = Workspace::new("keys") else {
        return;
    };
    let first = ws.build(STAMPED);
    assert!(first.success, "{}", first.stderr);
    assert_eq!(bep::check_complete(&first.events), Ok(()));
    // Two genrules and the status action, which is Bazel's `internal` one.
    assert_eq!(
        (first.total(), first.internal()),
        (3, 1),
        "{}",
        first.stderr
    );
    assert_eq!(first.value("STABLE_GIT_COMMIT").as_deref(), Some("abc123"));
    assert_eq!(first.value("BUILD_SCM_STATUS").as_deref(), Some("clean"));
    assert_eq!(first.value("BUILD_EMBED_LABEL").as_deref(), Some("r1"));
    for key in ["BUILD_USER", "BUILD_HOST", "BUILD_TIMESTAMP"] {
        assert!(first.value(key).is_some(), "{key} in {:?}", first.status());
    }

    // Nothing changed: only the status action runs.
    let again = ws.build(STAMPED);
    assert_eq!(
        (again.total(), again.internal()),
        (1, 1),
        "{}",
        again.stderr
    );
    assert_eq!(again.stdout, first.stdout);

    // A stable value changed: the stamped target rebuilds, the plain one does not.
    ws.status_env("def456", "clean");
    let stable = ws.build(STAMPED);
    assert_eq!(
        (stable.total(), stable.internal()),
        (2, 1),
        "{}",
        stable.stderr
    );
    assert_ne!(stable.stdout, first.stdout);
    assert_eq!(stable.value("STABLE_GIT_COMMIT").as_deref(), Some("def456"));

    // Only a volatile value changed: nothing rebuilds.
    ws.status_env("def456", "dirty");
    let volatile = ws.build(STAMPED);
    assert_eq!(
        (volatile.total(), volatile.internal()),
        (1, 1),
        "{}",
        volatile.stderr
    );
    assert_eq!(volatile.stdout, stable.stdout);
    assert_eq!(volatile.value("BUILD_SCM_STATUS").as_deref(), Some("dirty"));
}

#[test]
#[ignore = "builds with a real Bazel release: cargo test -p buildx --test stamp -- --ignored"]
fn the_default_status_has_bazels_keys_and_unstamped_targets_ignore_it() {
    let Some(ws) = Workspace::new("default") else {
        return;
    };
    // `//:stamped` needs the command's keys, so only the plain target is built.
    let first = ws.build_targets("", "\"//:plain\"");
    assert!(first.success, "{}", first.stderr);
    let keys: Vec<String> = first.status().into_iter().map(|(k, _)| k).collect();
    assert_eq!(
        keys,
        [
            "BUILD_EMBED_LABEL",
            "BUILD_HOST",
            "BUILD_TIMESTAMP",
            "BUILD_USER",
            "FORMATTED_DATE"
        ]
    );
    assert_eq!((first.total(), first.internal()), (2, 1));
    let again = ws.build_targets("", "\"//:plain\"");
    assert_eq!(
        (again.total(), again.internal()),
        (1, 1),
        "{}",
        again.stderr
    );
}

#[test]
#[ignore = "builds with a real Bazel release: cargo test -p buildx --test stamp -- --ignored"]
fn a_failing_status_command_fails_the_build_with_bazels_message() {
    let Some(ws) = Workspace::new("failing") else {
        return;
    };
    let run = ws.build("workspace_status_command = \"tools/missing.sh\"");
    assert!(!run.success);
    assert!(
        run.stderr
            .contains("Failed to determine workspace status: Process exited with status 127"),
        "{}",
        run.stderr
    );
    assert_eq!(bep::check_complete(&run.events), Ok(()));
    assert_eq!(run.status(), Vec::new(), "an empty status, as Bazel posts");
    assert_eq!(
        bep::exit_code(&run.events),
        Some(("BUILD_FAILURE".into(), 1))
    );
}
