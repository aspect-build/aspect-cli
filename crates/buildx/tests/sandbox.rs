//! The sandboxed executor in real builds (ignored by default, they need a Bazel
//! release in the buildx cache): which actions the sandbox stops, which tags run
//! unsandboxed, the runner name in the build metrics, and that nothing is left
//! in the output base's `sandbox` directory.

use buildx::bep;

struct Workspace {
    dir: std::path::PathBuf,
}

impl Workspace {
    fn new(name: &str) -> Option<Workspace> {
        let home = std::path::PathBuf::from(std::env::var_os("HOME")?);
        let caches = if cfg!(target_os = "macos") {
            home.join("Library/Caches")
        } else {
            home.join(".cache")
        };
        if !caches.join("buildx/bazel/9.0.1/A-server.jar").exists() {
            eprintln!("skipped: no Bazel 9.0.1 release in the buildx cache");
            return None;
        }
        let dir =
            std::env::temp_dir().join(format!("buildx-sandbox-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("MODULE.bazel"), "module(name = \"fixture\")\n").unwrap();
        std::fs::write(dir.join(".bazelversion"), "9.0.1\n").unwrap();
        std::fs::write(dir.join("sibling.txt"), "sibling\n").unwrap();
        std::fs::write(
            dir.join("BUILD.bazel"),
            r#"genrule(name = "ok", outs = ["ok.txt"], cmd = "echo ok > $@")
genrule(name = "chain", srcs = [":ok"], outs = ["chain.txt"], cmd = "cat $< > $@ && echo chain >> $@")
genrule(name = "peek", outs = ["peek.txt"], cmd = "cat sibling.txt > $@")
genrule(name = "peek_no_sandbox", outs = ["peek_no_sandbox.txt"], cmd = "cat sibling.txt > $@", tags = ["no-sandbox"])
genrule(name = "peek_local", outs = ["peek_local.txt"], cmd = "cat sibling.txt > $@", local = 1)
"#,
        )
        .unwrap();
        Some(Workspace { dir })
    }

    /// Builds every fixture target with `executor` (Starlark source) and
    /// returns stdout, stderr and the build's event file.
    fn build(&self, executor: &str) -> (String, String, Vec<u8>) {
        let file = self.dir.join("out.bep");
        let _ = std::fs::remove_file(&file);
        let script = format!(
            r#"def _i(ctx: Context):
    results = ctx.build(["//:chain", "//:peek", "//:peek_no_sandbox", "//:peek_local"], executor = {executor}, events = [ctx.events.file({:?})])
    for label, r in results.items():
        ctx.output.print(label, "FAILED" if r.failures() else "ok")
main = buildx_main(impl = _i, cli_args = {{}})
"#,
            file.display().to_string()
        );
        std::fs::write(self.dir.join("build.star"), script).unwrap();
        let out = std::process::Command::new(env!("CARGO_BIN_EXE_buildx"))
            .arg("build.star:main")
            .current_dir(&self.dir)
            .env("XDG_CACHE_HOME", self.dir.join("cache"))
            .output()
            .unwrap();
        (
            String::from_utf8_lossy(&out.stdout).into_owned(),
            String::from_utf8_lossy(&out.stderr).into_owned(),
            std::fs::read(&file).unwrap_or_default(),
        )
    }

    /// Whether the output base still has a `sandbox` directory, which a
    /// finished build removes: `<cache>/bazel/_bazel_<user>/<hash>/sandbox`.
    fn sandbox_dir_left(&self) -> bool {
        std::fs::read_dir(self.dir.join("cache/bazel"))
            .into_iter()
            .flatten()
            .flatten()
            .flat_map(|root| {
                std::fs::read_dir(root.path())
                    .into_iter()
                    .flatten()
                    .flatten()
            })
            .any(|output_base| output_base.path().join("sandbox").exists())
    }
}

impl Drop for Workspace {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.dir);
    }
}

fn runners(events: &[u8]) -> Vec<(String, i32)> {
    let events = bep::read_events(events).unwrap();
    events
        .iter()
        .filter_map(|e| match &e.payload {
            Some(axl_proto::build_event_stream::build_event::Payload::BuildMetrics(m)) => {
                m.action_summary.as_ref()
            }
            _ => None,
        })
        .flat_map(|s| s.runner_count.iter().map(|r| (r.name.clone(), r.count)))
        .collect()
}

#[test]
#[ignore = "builds with a real Bazel release: cargo test -p buildx --test sandbox -- --ignored"]
fn the_sandbox_stops_undeclared_reads_and_honours_the_tags() {
    let Some(ws) = Workspace::new("explicit") else {
        return;
    };
    let (stdout, stderr, bep) = ws.build("ctx.execution.sandbox()");
    assert!(stdout.contains("//:chain\") ok"), "{stdout}\n{stderr}");
    assert!(stdout.contains("//:peek\") FAILED"), "{stdout}\n{stderr}");
    assert!(
        stdout.contains("//:peek_no_sandbox\") ok"),
        "{stdout}\n{stderr}"
    );
    assert!(stdout.contains("//:peek_local\") ok"), "{stdout}\n{stderr}");
    let runners = runners(&bep);
    let sandboxed = runners
        .iter()
        .find(|(n, _)| n.ends_with("-sandbox"))
        .unwrap_or_else(|| panic!("no sandbox runner in {runners:?}"));
    assert_eq!(sandboxed.1, 3, "ok, chain and peek: {runners:?}");
    assert!(runners.contains(&("local".to_string(), 2)), "{runners:?}");
    assert!(!ws.sandbox_dir_left());
}

#[test]
#[ignore = "builds with a real Bazel release: cargo test -p buildx --test sandbox -- --ignored"]
fn the_default_is_the_sandbox_and_local_is_not() {
    let Some(ws) = Workspace::new("default") else {
        return;
    };
    let (stdout, stderr, _) = ws.build("None");
    assert!(stdout.contains("//:peek\") FAILED"), "{stdout}\n{stderr}");
    let local = Workspace::new("local").unwrap();
    let (stdout, stderr, bep) = local.build("ctx.execution.local()");
    assert!(stdout.contains("//:peek\") ok"), "{stdout}\n{stderr}");
    assert!(
        runners(&bep).iter().all(|(n, _)| !n.ends_with("-sandbox")),
        "{:?}",
        runners(&bep)
    );
}
