//! `BazelTrait.exec_log_event` fires, for real, from every built-in task that
//! drives Bazel.
//!
//! The hook's lifecycle is three calls a task has to make in the right places —
//! `open` before the spawn, `pump` in the drain loop, `close` before `wait()`
//! (see `builtins/aspect/bazel/exec_log.axl`) — hand-written at nine call sites
//! across seven files, two of them (`run.axl`, `delivery.axl`) wiring more than
//! one. Each one fails *quietly*: a missing `open` leaves the log off, a missing
//! `pump` means hooks only fire at the end of the build, and a missing `close` or
//! one moved after `wait()` loses entries. Nothing about the AXL unit tests,
//! which exercise the dispatcher against synthetic handles, would notice any of
//! that.
//!
//! So each case here runs the real CLI against a scratch workspace whose
//! `.aspect/config.axl` registers an `exec_log_event` hook, with basil standing
//! in for Bazel, and asserts on what the hook actually received:
//!
//!   * **every entry, in order** — basil's log is 3000 entries, past the decoded
//!     channel's 1000-entry capacity, so a handle that was never opened (0), a
//!     log repointed away from the reader (0) or a drain that stopped early
//!     (< 3000) are all distinguishable from a complete one.
//!   * **during the build, not after it** — `BASIL_EXECLOG_PUMP_HANDSHAKE` makes
//!     basil hold back the final build event until the hook has seen its first
//!     entry, so `pumped` in the status file means the dispatch happened inside
//!     the task's drain loop. A task that only drains after the event stream
//!     closes cannot get there, and the status reads `timeout`.
//!   * **under a phase that names the invocation** — every call-site case
//!     asserts the `ctx.task.current_phase().name` the hook read, because that
//!     is the only thing telling a hook which of a task's Bazel invocations it
//!     is in, and it is live-task state no unit test can produce. `delivery`
//!     contributes a case per wired phase (`build`, `deliver`), and
//!     `a_hook_tells_one_invocation_of_a_task_from_the_next` pins two
//!     invocations apart inside one task.
//!
//! The retry case covers the third seam: a hook registered by a `build_start`
//! hook must still see attempt 0's entries (the ordering bug `open`-before-
//! `build_start` would reintroduce), and every attempt must get a handle of its
//! own, since a handle binds to one build.
//!
//! Whether a task *succeeds* is beside the point and deliberately not asserted:
//! basil builds nothing, so `run`, `lint`, `format`, `gazelle` and `delivery`
//! all fail once they look for an output. They fail after their Bazel
//! invocation, which is the part under test.

mod common;

use common::aspect_cli;
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::OnceLock;
use std::time::{Duration, Instant};

/// Entries per log basil writes for the scenarios used here, from its
/// `execlog_entries`. Each attempt numbers its own log from 1.
const BLOCK: u32 = 3000;

/// Filename, under a case's directory, of the `compact_file` sink
/// [`Fixture::trait_sink`] registers.
const TRAIT_SINK: &str = "trait-sink.binpb.zst";

/// Bound on a single `aspect` invocation. Generous: it is here to turn a hang
/// (a `close` that waits on a producer joined by `wait()`) into a failure, not
/// to bound a healthy run, which takes under two seconds.
const RUN_TIMEOUT: Duration = Duration::from_secs(120);

/// What the fixture hook recorded, parsed from its report file.
struct Report {
    /// Entries dispatched to the hook across the whole task.
    total: u32,
    /// Entries whose `id` was not the next one expected within its attempt.
    /// Non-zero means a gap, a repeat or a reorder.
    gaps: u32,
    /// The distinct `ctx.task.current_phase().name` values the hook saw, in
    /// first-seen order — how a hook tells one Bazel invocation of a task from
    /// the next. `<none>` stands in for a phase the runtime had not opened yet.
    phases: Vec<String>,
    /// `ctx.task.name`, the other half of the answer: which task's invocation
    /// this was, for a hook several tasks registered.
    task: String,
    /// basil's side of the pump handshake: `pumped`, `timeout`, or `None` for
    /// a scenario that does not run one.
    handshake: Option<String>,
}

/// How a case's fixture config differs from the default one.
#[derive(Clone, Copy, Default)]
struct Fixture {
    /// Register the hook from a `build_start` hook instead of at config time,
    /// which is the shape the attempt-0 ordering bug needed: `open` has to read
    /// the trait after `build_start`, not before.
    late: bool,
    /// Clear the trait's `build_event` hooks from `build_start`, leaving
    /// `bazel/invocation.axl` with no BES event iterator to loop over.
    no_bes: bool,
    /// Register a `compact_file` sink on `BazelTrait.execution_log_sinks`, at
    /// [`TRAIT_SINK`] under the case's directory. Whether that file exists
    /// afterwards records which invocations Bazel was told to write it for.
    trait_sink: bool,
}

/// The `.aspect/config.axl` a case runs under: one `exec_log_event` hook that
/// checks each entry's `id` against the next one expected, notes the task phase
/// the entry arrived under, and republishes a one-line report whenever a whole
/// log has arrived.
///
/// The report is written from the hook rather than from `build_end`, because
/// `run --watch` never reaches a `build_end` and the point is to cover it too.
/// Writing it per completed log (rather than per entry) keeps 3000 dispatches
/// from becoming 3000 file writes.
///
/// The phase is read from the hook's own `TaskContext` — not the config-time
/// `ctx` closed over for `fs` — because that is the context a real hook gets and
/// the only one carrying a live task.
fn fixture(report: &Path, handshake: &Path, scenario: &str, f: Fixture) -> String {
    let mut setup = String::new();
    if f.trait_sink {
        let path = report.with_file_name(TRAIT_SINK);
        setup.push_str(&format!(
            "    t.execution_log_sinks.append(bazel.execution_log.compact_file(path = {:?}))\n",
            path.display().to_string(),
        ));
    }
    if f.no_bes {
        // Runs before `_do_spawn` reads `bool(trait.build_event)`, which is what
        // decides whether there is an event iterator at all. Something always
        // registers one in practice — the Artifact Upload feature does,
        // unconditionally — so this is how a task with no BES consumer is reached.
        setup.push_str("    t.build_start.append(lambda _ctx: t.build_event.clear())\n");
    }
    setup.push_str(if f.late {
        // `build_start` fires on attempt 0 only, so this appends exactly once.
        "    t.build_start.append(lambda _ctx: t.exec_log_event.append(_hook()))\n"
    } else {
        "    t.exec_log_event.append(_hook())\n"
    });
    format!(
        r#""""Test fixture: record every execution log entry the task dispatches."""

load("@aspect//traits.axl", "BazelTrait", "ExecLogHook")

_BLOCK = {block}
_REPORT = "{report}"
_HANDSHAKE = "{handshake}"

def config(ctx: ConfigContext):
    state = {{"total": 0, "gaps": 0, "phases": []}}

    def _on_entry(task_ctx, entry):
        if entry.id != (state["total"] % _BLOCK) + 1:
            state["gaps"] += 1
        state["total"] += 1
        phase = task_ctx.task.current_phase()
        name = phase.name if phase else "<none>"
        if name not in state["phases"]:
            state["phases"].append(name)
        if state["total"] == 1:
            # basil is holding back the last build event until this appears.
            ctx.std.fs.create(_HANDSHAKE).write("x")
        if state["total"] % _BLOCK == 0:
            ctx.std.fs.create(_REPORT).write("total=%d gaps=%d phases=%s task=%s\n" % (
                state["total"],
                state["gaps"],
                ",".join(state["phases"]),
                task_ctx.task.name,
            ))

    def _hook():
        return ExecLogHook(on_entry = _on_entry)

    t = ctx.traits[BazelTrait]
{setup}    t.extra_flags.append("--scenario={scenario}")
"#,
        block = BLOCK,
        report = report.display(),
        handshake = handshake.display(),
        scenario = scenario,
        setup = setup,
    )
}

/// Locate basil, the fake `bazel` the CLI is pointed at.
///
/// Bazel sets `BASIL_BIN` from the `rust_test` rule's `env` via
/// `$(rootpath //crates/basil)`, relative to the runfiles root that is a
/// Bazel-run test's cwd. Under cargo, `CARGO_BIN_EXE_*` covers only this
/// crate's own binaries, so basil is built on demand and found next to this
/// test executable — the same recursive-cargo pattern `axl_runtime::test` uses.
fn basil_bin() -> &'static str {
    static BIN: OnceLock<String> = OnceLock::new();
    BIN.get_or_init(|| {
        if let Ok(p) = std::env::var("BASIL_BIN") {
            return std::fs::canonicalize(&p)
                .unwrap_or_else(|e| panic!("BASIL_BIN={p:?} not found: {e}"))
                .to_string_lossy()
                .into_owned();
        }
        let test_exe = std::env::current_exe().expect("current_exe");
        let mut path: PathBuf = test_exe.parent().expect("test exe parent").to_path_buf();
        if path.ends_with("deps") {
            path.pop();
        }
        path.push("basil");
        if !path.exists() {
            let cargo = std::env::var("CARGO").unwrap_or_else(|_| "cargo".to_string());
            let workspace_root = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
                .parent()
                .and_then(Path::parent)
                .expect("workspace root above crates/aspect-cli")
                .to_path_buf();
            let status = Command::new(&cargo)
                .args(["build", "--quiet", "-p", "basil"])
                .current_dir(&workspace_root)
                .status()
                .expect("invoking cargo to build basil");
            assert!(status.success(), "cargo build -p basil failed: {status}");
        }
        assert!(path.exists(), "basil not found at {}", path.display());
        path.to_string_lossy().into_owned()
    })
}

/// The environment a case runs under: the developer's own, minus everything
/// that would make the result depend on where the test ran.
///
/// `ASPECT_*` would override the fixture's config, and the CI identity
/// variables steer the built-in tasks into their CI code paths (status checks,
/// runner detection) — the same filtering `tools/cache-diff-integration.py`
/// does for the same reason. `HOME` and `PATH` stay: the CLI resolves its
/// builtins cache under `HOME`.
fn base_env() -> HashMap<String, String> {
    const DROP_PREFIXES: &[&str] = &[
        "ASPECT_",
        "BASIL_",
        "BUILDKITE_",
        "CIRCLE",
        "GITHUB_",
        "GITLAB_",
    ];
    std::env::vars()
        .filter(|(k, _)| k != "CI" && !DROP_PREFIXES.iter().any(|p| k.starts_with(p)))
        .collect()
}

/// A scratch workspace plus the live process that stands in for the Bazel
/// daemon, dropped together when the case ends.
///
/// The stand-in is what keeps the execution log readable: the reader asks the
/// pid `bazel info server_pid` reported whether more bytes are coming, and
/// basil's own `info` process is already reaped by then — a dead holder reads a
/// not-yet-created log as "the writer is gone" and ends the stream empty. A
/// live process that holds nothing open is the right answer, and passing its pid
/// through the child's environment (rather than this test process's) is what
/// lets the cases run in parallel.
struct Case {
    dir: tempfile::TempDir,
    daemon: Child,
    /// The pid handed to basil as the log's nominated holder. The live stand-in
    /// unless a test replaced it to model a server that went away.
    holder: Option<u32>,
}

impl Drop for Case {
    fn drop(&mut self) {
        let _ = self.daemon.kill();
        let _ = self.daemon.wait();
    }
}

impl Case {
    fn new(scenario: &str, f: Fixture, bazelrc: Option<&str>) -> Self {
        let dir = tempfile::tempdir().expect("temp dir");
        let root = dir.path();
        std::fs::write(root.join("MODULE.bazel"), "").expect("MODULE.bazel");
        std::fs::write(root.join("MODULE.aspect"), "").expect("MODULE.aspect");
        if let Some(rc) = bazelrc {
            std::fs::write(root.join(".bazelrc"), rc).expect(".bazelrc");
        }
        std::fs::create_dir(root.join(".aspect")).expect(".aspect");
        std::fs::write(
            root.join(".aspect/config.axl"),
            fixture(&root.join("report.txt"), &root.join("pumped"), scenario, f),
        )
        .expect("config.axl");

        let daemon = Command::new("/bin/sleep")
            .arg("600")
            .spawn()
            .expect("spawning the daemon stand-in");
        Self {
            dir,
            daemon,
            holder: None,
        }
    }

    /// Nominate a reaped pid as the execution log's holder, modelling the server
    /// that Bazel killed out from under the reader.
    ///
    /// The reader asks that pid whether more bytes are coming; dead, it reads a
    /// log that has not appeared yet as one that never will. basil still writes
    /// its log and still exits 0, so this is the one combination that is worth a
    /// warning: a successful build with nothing delivered.
    fn with_dead_holder(mut self) -> Self {
        let mut gone = Command::new("/bin/sh")
            .args(["-c", "exit 0"])
            .spawn()
            .expect("spawning a process to reap");
        self.holder = Some(gone.id());
        gone.wait().expect("reaping it");
        self
    }

    fn command(&self, args: &[&str]) -> Command {
        let root = self.dir.path();
        let mut cmd = Command::new(aspect_cli());
        cmd.args(args)
            .current_dir(root)
            .env_clear()
            .envs(base_env())
            .env("BAZEL_REAL", basil_bin())
            .env(
                "BASIL_SERVER_PID",
                self.holder.unwrap_or_else(|| self.daemon.id()).to_string(),
            )
            .env("BASIL_EXECLOG_PUMP_HANDSHAKE", root.join("pumped"))
            .env("ASPECT_CREDENTIALS_FILE", root.join("credentials.json"))
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        cmd
    }

    /// Run to completion and hand back the process output, for a case whose
    /// subject is what the CLI said rather than what the hook received.
    fn raw(&self, args: &[&str]) -> std::process::Output {
        self.command(args)
            .output()
            .unwrap_or_else(|e| panic!("running `aspect {}`: {e}", args.join(" ")))
    }

    /// Run to completion and report what the hook saw.
    fn run(&self, args: &[&str]) -> Report {
        let output = self
            .command(args)
            .output()
            .unwrap_or_else(|e| panic!("running `aspect {}`: {e}", args.join(" ")));
        self.report(args, Some(&output))
    }

    /// Run until the hook has published a report, then stop the CLI.
    ///
    /// For `run --watch`, whose session ends only on Ctrl-C: there is no exit to
    /// wait for, so the report is the completion signal.
    fn run_until_reported(&self, args: &[&str]) -> Report {
        let mut child = self
            .command(args)
            .spawn()
            .unwrap_or_else(|e| panic!("spawning `aspect {}`: {e}", args.join(" ")));
        let deadline = Instant::now() + RUN_TIMEOUT;
        while Instant::now() < deadline && !self.dir.path().join("report.txt").exists() {
            if child.try_wait().expect("try_wait").is_some() {
                break;
            }
            std::thread::sleep(Duration::from_millis(50));
        }
        let _ = child.kill();
        let _ = child.wait();
        self.report(args, None)
    }

    fn report(&self, args: &[&str], output: Option<&std::process::Output>) -> Report {
        let root = self.dir.path();
        let context = || match output {
            Some(o) => format!(
                "`aspect {}` exited with {:?}\n--- stdout ---\n{}\n--- stderr ---\n{}",
                args.join(" "),
                o.status.code(),
                String::from_utf8_lossy(&o.stdout),
                String::from_utf8_lossy(&o.stderr),
            ),
            None => format!("`aspect {}`", args.join(" ")),
        };
        let text = std::fs::read_to_string(root.join("report.txt")).unwrap_or_else(|_| {
            panic!(
                "the exec_log_event hook never received a whole log; it reports once per \
                 {BLOCK} entries.\n{}",
                context()
            )
        });
        let raw = |name: &str| -> &str {
            text.split_whitespace()
                .find_map(|f| f.strip_prefix(name))
                .unwrap_or_else(|| panic!("no {name} field in report {text:?}"))
        };
        let field = |name: &str| -> u32 {
            raw(name)
                .parse()
                .unwrap_or_else(|_| panic!("unparseable {name} field in report {text:?}"))
        };
        Report {
            total: field("total="),
            gaps: field("gaps="),
            phases: raw("phases=").split(',').map(str::to_string).collect(),
            task: raw("task=").to_string(),
            handshake: std::fs::read_to_string(root.join("pumped.status"))
                .ok()
                .map(|s| s.trim().to_string()),
        }
    }
}

/// Assert the hook saw `logs` whole logs, each entry once and in order.
fn assert_complete(case: &str, report: &Report, logs: u32) {
    assert_eq!(
        report.total,
        BLOCK * logs,
        "{case}: the hook should have received {} entries",
        BLOCK * logs,
    );
    assert_eq!(
        report.gaps, 0,
        "{case}: every entry should arrive once, in order, numbered from 1 per attempt",
    );
}

/// Every site, under both halves of the lifecycle.
///
/// The two scenarios differ only in *when* basil publishes the log, which is
/// what separates the two verbs that read it. Published before the event stream
/// ends, only `pump` can see it during the build — and basil holds the last
/// event back until it does, so `pumped` is proof rather than a race. Published
/// after the stream ends, nothing in the drain loop can reach it and only
/// `close` can, so an empty hook means the end-of-build drain is gone or runs
/// after the `wait()` that releases its handle.
///
/// `phase` is the task phase every entry of this site must arrive under. It is
/// asserted at each site rather than once, because the phase is what a hook has
/// to identify the invocation with, and a site that opened no phase before
/// spawning Bazel would hand it `<none>` — readable only here, where a live task
/// is driving the hook.
fn assert_site(name: &str, args: &[&str], bazelrc: Option<&str>, watch: bool, phase: &str) {
    let drive = |case: &Case| -> Report {
        if watch {
            case.run_until_reported(args)
        } else {
            case.run(args)
        }
    };

    let pumped = Case::new("execlog_beyond_capacity", Fixture::default(), bazelrc);
    let report = drive(&pumped);
    assert_complete(name, &report, 1);
    assert_eq!(
        report.handshake.as_deref(),
        Some("pumped"),
        "{name}: the first entry must reach the hook before the build ends, which is \
         what calling `exec_log.pump` in the task's drain loop does",
    );
    assert_eq!(
        report.phases,
        vec![phase.to_string()],
        "{name}: every entry should arrive under the `{phase}` phase, which is what \
         `ctx.task.current_phase()` has to give a hook that needs to know which \
         invocation it is in",
    );

    let drained = Case::new("execlog_after_bes", Fixture::default(), bazelrc);
    let report = drive(&drained);
    assert_complete(name, &report, 1);
    assert_eq!(
        report.phases,
        vec![phase.to_string()],
        "{name}: the entries `exec_log.close` carries after the event stream ends are \
         still inside the phase that spawned Bazel",
    );
}

/// `bazel/invocation.axl`, the call site `build` and `test` share.
#[test]
fn the_build_task_dispatches_every_entry() {
    assert_site("build", &["build", "//..."], None, false, "build");
}

#[test]
fn the_test_task_dispatches_every_entry() {
    assert_site("test", &["test", "//..."], None, false, "test");
}

/// `run.axl`'s `_run_impl`.
#[test]
fn the_run_task_dispatches_every_entry() {
    assert_site("run", &["run", "//fixture:target"], None, false, "build");
}

/// `run.axl`'s `_build_once`, the second of that file's two call sites, reached
/// only by the watch session's first build. A watch session ends on Ctrl-C, so
/// the hook's report is what this one waits for instead of an exit.
#[test]
fn the_run_watch_task_dispatches_every_entry() {
    assert_site(
        "run --watch",
        &["run", "--watch", "//fixture:target"],
        None,
        true,
        "build",
    );
}

#[test]
fn the_lint_task_dispatches_every_entry() {
    assert_site("lint", &["lint", "//..."], None, false, "lint");
}

#[test]
fn the_format_task_dispatches_every_entry() {
    assert_site("format", &["format"], None, false, "build");
}

#[test]
fn the_gazelle_task_dispatches_every_entry() {
    assert_site("gazelle", &["gazelle"], None, false, "build");
}

#[test]
fn the_warming_task_dispatches_every_entry() {
    assert_site(
        "ci warming",
        &["ci", "warming", "//..."],
        None,
        false,
        "populate",
    );
}

/// `delivery.axl`'s `_get_output_shas` — the phase-1 build, which runs only
/// when there is a remote cache to read digests from and change detection or a
/// dry run to read them for. Its phase-2 build has no hook handle, so the hook
/// still sees exactly one log; phase 3 is skipped by a bare `--dry-run` and gets
/// its own case below.
#[test]
fn the_delivery_task_dispatches_every_entry() {
    assert_site(
        "delivery",
        &[
            "delivery",
            "--mode=always",
            "--dry-run",
            "--track-state=false",
            "--commit-sha=0000000000000000000000000000000000000000",
            "//fixture:target",
        ],
        Some("build --remote_cache=grpc://127.0.0.1:1\n"),
        false,
        "build",
    );
}

/// `delivery.axl`'s phase 3, the release build the dispatch reads its artifacts
/// from — a second call site in the same file, and the one a per-file wiring
/// check cannot distinguish from phase 1.
///
/// Reaching it without a remote cache is what makes the case a single log:
/// `--mode=always --dry-run=build --track-state=false` is the one combination
/// that drops phases 1 and 2 when no cache is configured (see the combination
/// matrix in `delivery.axl`) while `=build` keeps the delivery build. So the
/// 3000 entries here are phase 3's own, and `deliver` is the phase they arrived
/// under — not `build`, which would mean phase 1 had run after all.
#[test]
fn the_delivery_release_build_dispatches_every_entry() {
    assert_site(
        "delivery --dry-run=build",
        &[
            "delivery",
            "--mode=always",
            "--dry-run=build",
            "--track-state=false",
            "--commit-sha=0000000000000000000000000000000000000000",
            "//fixture:target",
        ],
        None,
        false,
        "deliver",
    );
}

/// A hook can tell one Bazel invocation of a task from the next.
///
/// This is the question a hook has to answer before it can attribute anything:
/// `aspect delivery` drives Bazel three times, and `build` retries drive it up
/// to three, so "which invocation am I in" is not answerable from an entry —
/// entries carry no invocation of their own. `ctx.task.current_phase().name` is
/// the answer, and a retry is the sharpest case for it: two invocations, one
/// task, one hook, no `build_start` in between.
///
/// The two names are the contract, not just two different strings: the phase the
/// task opens per attempt is `build` then `build_retry_2`
/// (`private/lib/bazel_results.axl` builds the second from the attempt index), so
/// a hook aggregating per invocation keys on exactly this. `ctx.task.name` is
/// the other half, for a hook that several tasks registered; its suffix is
/// generated per run, so only the prefix is assertable.
#[test]
fn a_hook_tells_one_invocation_of_a_task_from_the_next() {
    let case = Case::new("execlog_retryable_failure", Fixture::default(), None);
    let report = case.run(&["build", "--bazel-retry-attempts=2", "//..."]);
    assert_complete("build --bazel-retry-attempts=2", &report, 2);
    assert_eq!(
        report.phases,
        vec!["build".to_string(), "build_retry_2".to_string()],
        "each attempt opens its own phase before spawning Bazel, so the phase name is \
         what separates attempt 0's entries from attempt 1's",
    );
    assert!(
        report.task.starts_with("build-"),
        "the hook should also be able to name the task it fired for; got {:?}",
        report.task,
    );
}

/// With no BES event stream, every entry still arrives — at the end of the
/// build rather than during it.
///
/// `bazel/invocation.axl` pumps from `on_event`, so its loop runs only when
/// there is an event iterator to loop over, and an `exec_log_event` hook does
/// not create one. Today something always does (the Artifact Upload feature
/// registers a `build_event` hook unconditionally, which is why every case above
/// reports `pumped`), but that is a coincidence of the default feature set, not
/// a guarantee this hook can rely on — so the contract is completeness, not
/// timeliness, and this pins the half that has to hold either way.
///
/// basil writes no event file at all here, so there is no handshake to record:
/// `None` is the assertion that the task had no event loop, and the 3000 entries
/// beside it are what `close` carried on its own.
#[test]
fn entries_arrive_without_a_bes_event_stream_to_pump_from() {
    let case = Case::new(
        "execlog_beyond_capacity",
        Fixture {
            no_bes: true,
            ..Fixture::default()
        },
        None,
    );
    let report = case.run(&["build", "//..."]);
    assert_complete("build with no BES stream", &report, 1);
    assert_eq!(
        report.handshake, None,
        "with no event stream there is no drain loop to pump from, so the whole \
         log must come from `exec_log.close`",
    );
}

/// The one shape worth warning about: bazel succeeded and produced no log.
///
/// That is what a nominated holder which died out from under the reader looks
/// like — every hook fires zero times, every sink writes nothing, and without the
/// warning the build is indistinguishable from one whose actions were all cached.
#[test]
fn a_successful_build_that_produced_no_log_warns() {
    let case = Case::new("execlog_beyond_capacity", Fixture::default(), None).with_dead_holder();
    let out = case.raw(&["build", "//..."]);
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert_eq!(
        out.status.code(),
        Some(0),
        "the build itself should still succeed\n{stderr}",
    );
    assert!(
        stderr.contains("wrote no execution log"),
        "a successful build that delivered no entries must say so:\n{stderr}",
    );
}

/// And the shape that must stay quiet: a failed invocation.
///
/// Bazel writes the compact log only once a command gets past target-pattern
/// parsing and package loading, so a mistyped target or an unparseable BUILD file
/// leaves none — nonzero exit, nothing wrong with the CLI, and Bazel has already
/// printed the reason. Warning there put a line about a temp path the user cannot
/// act on *after* bazel's own error, reading as though the CLI had broken.
#[test]
fn a_failed_build_without_a_log_stays_quiet() {
    let case = Case::new("rejects_command_line", Fixture::default(), None);
    let out = case.raw(&["build", "//..."]);
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert_ne!(
        out.status.code(),
        Some(0),
        "this scenario models bazel rejecting the command line\n{stderr}",
    );
    assert!(
        !stderr.contains("wrote no execution log"),
        "a failed invocation writes no log by design; warning about it crowds out \
         bazel's own error:\n{stderr}",
    );
}

/// Delivery's phase 3 dispatches to hooks without writing the trait's file
/// sinks.
///
/// Those two things travel together everywhere else — `exec_log.open` returns
/// the trait's sinks *and* the hook handle in one list — and delivery is the one
/// task where handing phase 3 both is a bug. It fires `build_end` straight after
/// phase 2, ~200 lines before phase 3 spawns, and the artifact uploader's
/// `build_end` uploads the execution log it asked for and then deletes it,
/// precisely because a leftover file leaks its highest-risk artifact: action
/// command lines and environment variables, unredacted. Phase 3 writing the
/// trait's sinks therefore re-created a path that had just been uploaded and
/// deleted, leaving the file on disk with nothing left to claim it.
///
/// What this asserts is the absence of that file after a run that reached phase
/// 3, with the hook's own 3000 entries beside it as proof phase 3 really ran —
/// so it fails if phase 3 is ever handed `xl.sinks` again.
///
/// It drives phase 3 alone, because phase 1 and phase 3 in one invocation is not
/// reachable here: phase 3 is gated behind phases 1+2 succeeding, and phase 2
/// needs a real remote cache and a real `--remote_grpc_log`, neither of which a
/// fake Bazel can produce. The invariant does not depend on phase 1 having run —
/// the upload-and-delete happens in `build_end` either way.
#[test]
fn the_delivery_release_build_does_not_write_the_traits_sinks() {
    let case = Case::new(
        "execlog_beyond_capacity",
        Fixture {
            trait_sink: true,
            ..Fixture::default()
        },
        None,
    );
    let report = case.run(&[
        "delivery",
        "--mode=always",
        "--dry-run=build",
        "--track-state=false",
        "--commit-sha=0000000000000000000000000000000000000000",
        "//fixture:target",
    ]);
    assert_complete("delivery phase 3", &report, 1);
    assert_eq!(
        report.phases,
        vec!["deliver".to_string()],
        "this case is meant to reach phase 3 and nothing else",
    );

    let sink = case.dir.path().join(TRAIT_SINK);
    assert!(
        !sink.exists(),
        "phase 3 wrote the trait's execution-log sink at {}. Delivery uploads and \
         deletes that file in `build_end`, which has already run by then, so the \
         re-created file is left behind unclaimed — and it holds action command \
         lines and environment variables.",
        sink.display(),
    );
}

/// Retries, the seam `_test_open_is_fresh_per_call` covers only in the unit.
///
/// The hook is registered by a `build_start` hook, which fires on attempt 0
/// only, so the 3000 entries of the first log are the ones an `open` resolved
/// before `build_start` would have missed — it would see 3000 in total, from
/// attempt 1, rather than 6000. The second 3000 are numbered from 1 again,
/// which a reused handle could not deliver: it is bound to the first build.
#[test]
fn every_retry_attempt_dispatches_its_own_log_to_a_build_start_hook() {
    let case = Case::new(
        "execlog_retryable_failure",
        Fixture {
            late: true,
            ..Fixture::default()
        },
        None,
    );
    let report = case.run(&["build", "--bazel-retry-attempts=2", "//..."]);
    assert_complete("build --bazel-retry-attempts=2", &report, 2);
    assert_eq!(
        report.handshake.as_deref(),
        Some("pumped"),
        "the hook a build_start hook registered must receive attempt 0's entries \
         during attempt 0, not only once the build is over",
    );
}
