//! Ctrl-C reaches only the terminal's foreground process group, so a child a
//! task starts in its own group (as `aspect run --watch` does) never sees it.
//! aspect-cli must interrupt that group on shutdown and let it clean up rather
//! than exit and orphan it.

#![cfg(unix)]

mod common;

use common::aspect_cli;
use std::path::Path;
use std::process::{Command, Stdio};
use std::thread;
use std::time::{Duration, Instant};

const TASK: &str = r#"
load("@std//time.axl", "sleep_iter")

def _impl(ctx):
    ctx.std.process.command("sh").args([
        "-c",
        "trap 'touch \"$MARKER\"; exit 0' INT; echo $$ > \"$READY\"; while :; do sleep 0.1; done",
    ]).process_group(0).spawn()
    for _ in sleep_iter(100):
        pass

grouped = task(summary = "Test fixture.", implementation = _impl)
"#;

fn wait_for(deadline: Duration, mut done: impl FnMut() -> bool) -> bool {
    let deadline = Instant::now() + deadline;
    while Instant::now() < deadline {
        if done() {
            return true;
        }
        thread::sleep(Duration::from_millis(20));
    }
    done()
}

fn group_exists(pgid: i32) -> bool {
    unsafe { libc::kill(-pgid, 0) == 0 }
}

#[test]
fn sigint_interrupts_a_task_child_in_its_own_process_group() {
    let dir = tempfile::tempdir().expect("temp dir");
    std::fs::write(dir.path().join("MODULE.bazel"), "").expect("MODULE.bazel");
    let aspect = dir.path().join(".aspect");
    std::fs::create_dir(&aspect).expect(".aspect");
    std::fs::write(aspect.join("grouped.axl"), TASK).expect("task fixture");
    let ready = dir.path().join("ready");
    let marker = dir.path().join("interrupted");

    let mut cli = Command::new(aspect_cli())
        .arg("grouped")
        .current_dir(dir.path())
        .env(
            "ASPECT_CREDENTIALS_FILE",
            dir.path().join("credentials.json"),
        )
        .env("READY", &ready)
        .env("MARKER", &marker)
        .env_remove("ASPECT_DEBUG")
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .spawn()
        .expect("spawning aspect");

    let read_pgid = |path: &Path| std::fs::read_to_string(path).ok()?.trim().parse().ok();
    let started = wait_for(Duration::from_secs(30), || read_pgid(&ready).is_some());
    if !started {
        let _ = cli.kill();
        panic!("the grouped child never started");
    }
    let pgid: i32 = read_pgid(&ready).unwrap();

    unsafe { libc::kill(cli.id() as i32, libc::SIGINT) };
    let output = cli.wait_with_output().expect("waiting for aspect");
    let stderr = String::from_utf8_lossy(&output.stderr);

    assert_eq!(output.status.code(), Some(130), "stderr:\n{stderr}");
    assert!(
        marker.exists(),
        "the child never handled SIGINT\nstderr:\n{stderr}"
    );
    if !wait_for(Duration::from_secs(2), || !group_exists(pgid)) {
        unsafe { libc::kill(-pgid, libc::SIGKILL) };
        panic!("process group {pgid} outlived aspect\nstderr:\n{stderr}");
    }
}
