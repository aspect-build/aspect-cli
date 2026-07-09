mod builtins;
mod cmd;
mod credential_helper;
mod helpers;
mod trace;
mod trace_buffer;

use std::process::ExitCode;
use std::time::Duration;

use aspect_telemetry::{cargo_pkg_short_version, do_not_track, send_telemetry};
use axl_runtime::bazel_live;
use axl_runtime::ci::on_recognized_ci;
use axl_runtime::eval::{Loader, ModuleEnv, MultiPhaseEval};
use axl_runtime::module::{AXL_ROOT_MODULE_NAME, Mod};
use axl_runtime::module::{DiskStore, ModEvaluator};
use tokio::task;
use tokio::task::spawn_blocking;
use tracing::info_span;

use crate::cmd::Cmd;
use crate::helpers::{
    find_aspect_root, find_bazel_root, find_git_root, get_default_axl_search_paths, search_sources,
};

// Must use a multi thread runtime with at least 3 threads for following reasons;
//
// Main thread (1) which drives the async runtime and all the other machinery shall
// not be starved of cpu time to perform async tasks, its sole purpose is to
// execute Rust code that drives the async runtime.
//
// Starlark thread (2) for command execution that is spawned via spawn_blocking will allow Starlark
// code run on a blocking thread pool separate from the threads that drive the async work.
//
// On the other hand, all the other async tasks, including those spawned by Starlark
// async machinery get to run on any of these worker threads (3+) until they are ready.
//
// As a special exception the build event machinery and build event sinks get
// their own threads (3+) to react to IO streams in a timely manner.
//
// TODO: create a diagram of how all this ties together.
#[tokio::main(flavor = "multi_thread", worker_threads = 3)]
async fn run() -> Result<ExitCode, anyhow::Error> {
    // Spawn the OS shutdown-signal handler before anything else can
    // acquire long-running resources. Catches SIGINT / SIGTERM (the
    // signals CI runners and humans use to cancel a job), forwards
    // SIGINT to every live bazel client subprocess registered in
    // `bazel_live`, and force-exits aspect-cli after a grace period.
    //
    // Without this, a CI cancel can hit bazel at a moment it can't
    // gracefully recover from. Two known flakes — both rare per
    // invocation, but bad when they fire on a warm runner:
    //   1. *Potential sandbox-state corruption* (bazelbuild/bazel#23880):
    //      if the bazel server is SIGKILL'd mid-sandbox-cleanup, it can
    //      strand `_moved_trash_dir` in the sandbox base. Every
    //      subsequent invocation on that runner then crashes in
    //      `afterCommand` until the runner is cleaned up. The health
    //      check (`engine::bazel::health_check`) now removes the dir
    //      on detection, recovering automatically.
    //   2. *Potential orphaned bazel client*: the client outlives
    //      aspect-cli briefly while still holding the JVM-server lock;
    //      the next `aspect build` / `aspect test` on that runner hangs
    //      at "Running Bazel server needs to be killed" until the
    //      orphan exits on its own.
    install_shutdown_handler();

    if !do_not_track() {
        let _ = task::spawn(send_telemetry());
    }

    let mut _tracing = trace::init();
    let _root = info_span!(
        "root",
        version = cargo_pkg_short_version(),
        pid = std::process::id(),
    )
    .entered();

    let current_work_dir = std::env::current_dir()?;
    // `Env` requires both roots; cwd is the last-resort fallback when no
    // marker file exists anywhere up the tree.
    let aspect_root = find_aspect_root(&current_work_dir)
        .await
        .unwrap_or_else(|| current_work_dir.clone());
    let bazel_root = find_bazel_root(&current_work_dir)
        .await
        .unwrap_or_else(|| current_work_dir.clone());
    let git_root = find_git_root(&current_work_dir).await;

    let disk_store = DiskStore::new(aspect_root.clone());
    let mode = ModEvaluator::new(aspect_root.clone());

    let root_mod = mode.evaluate(AXL_ROOT_MODULE_NAME.to_string(), aspect_root.clone())?;
    let builtins = builtins::expand_builtins(aspect_root.clone(), disk_store.builtins_path())?;
    let module_roots = disk_store.expand_store(&root_mod, builtins).await?;

    let mut modules: Vec<Mod> = vec![];
    for (name, root) in module_roots {
        let r#mod = mode.evaluate(name, root)?;
        axl_runtime::trace!("module @{} at {:?}", r#mod.name, r#mod.root);
        modules.push(r#mod)
    }

    let search_paths = get_default_axl_search_paths(&current_work_dir, &aspect_root);
    let (scripts, configs) = search_sources(&search_paths).await?;

    // `_root` is entered on this thread; spawn_blocking moves work to a
    // different thread where the span stack is empty. Capture the span and
    // re-enter it on the worker so the phase spans nest under `root`.
    let parent_span = tracing::Span::current();
    let out = spawn_blocking(move || -> Result<ExitCode, anyhow::Error> {
        let _enter = parent_span.enter();
        let cli_version = cargo_pkg_short_version();

        ModuleEnv::with(|env| -> Result<ExitCode, anyhow::Error> {
            let loader = Loader::new(
                cli_version.clone(),
                aspect_root.clone(),
                bazel_root.clone(),
                git_root.clone(),
                &root_mod,
                &modules,
            );
            let mut mpe = MultiPhaseEval::new(env, &loader);

            // Phase 1: discover tasks and features.
            mpe.eval(&scripts, &root_mod, &modules)
                .map_err(anyhow::Error::from)?;

            // Phase 2: run config files.
            mpe.execute_configs(&configs, &root_mod)
                .map_err(anyhow::Error::from)?;

            // Build the CLI surface from current eval state.
            let cmd = Cmd {
                tasks: mpe.tasks(),
                features: mpe.features(),
                aspect_root: &aspect_root,
                modules: &modules,
            };
            let mut root_cmd = cmd.build(&cli_version)?;
            let mut cmd_for_help = root_cmd.clone();

            let matches = match root_cmd.try_get_matches_from_mut(std::env::args_os()) {
                Ok(m) => m,
                Err(err) => {
                    err.print().ok();
                    return Ok(ExitCode::from(err.exit_code() as u8));
                }
            };

            match matches.subcommand_name() {
                Some("version") => {
                    println!("{}", cargo_pkg_short_version());
                    return Ok(ExitCode::SUCCESS);
                }
                Some("help") => {
                    cmd_for_help.print_help()?;
                    return Ok(ExitCode::SUCCESS);
                }
                _ => {}
            }

            let dispatch = cmd.dispatch(matches)?;

            // `aspect axl test` is served by the native test runner. It borrows
            // the live loader to load every `*.test.axl` file through the normal
            // load path — so a test file's own `load(...)`s resolve — then runs
            // its `test_*` functions. This bypasses the ordinary feature/task
            // phases, which a self-contained test run does not need.
            {
                let tasks = mpe.tasks();
                if let Some(task) = tasks.get(dispatch.task_id) {
                    if task.name() == "test" && task.group().as_slice() == ["axl"] {
                        let paths = dispatch.string_list("paths");
                        let code = axl_runtime::engine::testing::run_tests(&loader, &paths)?;
                        return Ok(ExitCode::from(code as u8));
                    }
                }
            }

            // Print the "Running <task>" header before feature
            // implementations run so any diagnostic output from feature
            // initialization (auth WARNINGs, tip blocks, etc.) is
            // framed by the header.
            mpe.print_running_task_header(dispatch.task_id, &dispatch.task_key)
                .map_err(anyhow::Error::from)?;

            // Phase 3: run enabled feature impls.
            mpe.execute_features_with_args(|f, h| dispatch.feature_args(f, h))
                .map_err(anyhow::Error::from)?;

            // Phase 3.5: install exporters from any registered via
            // `ctx.telemetry.exporters.add(...)`. Replays buffered spans
            // and logs to them before phase 4 starts emitting task traces.
            // No-op (and disables further OTel work for the rest of the run)
            // if no exporter was registered.
            let exporters = mpe.drain_exporters();
            tokio::runtime::Handle::current().block_on(trace::install_late_exporters(exporters))?;

            // Phase 4: execute the selected task.
            let exit = mpe
                .execute_tasks_with_args(
                    dispatch.task_id,
                    dispatch.task_key.clone(),
                    dispatch.task_uuid.clone(),
                    dispatch.timing,
                    |t, h| dispatch.task_args(t, h),
                )
                .map_err(anyhow::Error::from)?;

            mpe.finish();
            Ok(ExitCode::from(exit.unwrap_or(0)))
        })
    });

    match out.await {
        Ok(result) => {
            drop(_root);
            drop(_tracing);
            result
        }
        Err(err) => panic!("{:?}", err),
    }
}

fn main() -> ExitCode {
    // Intercept the Bazel credential helper (`aspect get`) before the async
    // runtime and workspace discovery so it stays fast (see `credential_helper`).
    if credential_helper::is_invocation() {
        return match credential_helper::run() {
            Ok(()) => ExitCode::SUCCESS,
            Err(err) => {
                eprintln!("error: {err:?}");
                ExitCode::FAILURE
            }
        };
    }

    match run() {
        Ok(code) => code,
        Err(err) => {
            eprintln!("error: {err:?}");
            ExitCode::FAILURE
        }
    }
}

/// Tick between successive SIGINTs when we mimic bazel's 3-SIGINT
/// cancel protocol. Short — bazel's signal handler just needs the
/// signal delivered; it does its own dispatching from there.
const SIGINT_TICK: Duration = Duration::from_millis(150);

/// Time we wait for bazel clients to exit after the SIGINT burst before
/// escalating to SIGKILL.
///
/// Only used off-CI: on CI we never self-SIGKILL (see `run_shutdown_sequence`),
/// so this grace window only governs the interactive/local path, where SIGKILL
/// is the backstop for a hung client. 12s gives bazel's graceful cancel ample
/// room to finish on a developer's machine.
const SIGINT_GRACE: Duration = Duration::from_secs(12);

/// Time we wait after SIGKILL for the kernel to deliver the signal
/// and the process accounting to settle before we exit. SIGKILL
/// can't be ignored, but on busy systems the actual termination
/// (and reaping by init) can lag a beat. A short final wait keeps
/// us from racing the kernel and exiting before children are gone.
const POST_KILL_GRACE: Duration = Duration::from_secs(1);

/// Total wall time between receiving the OS signal and `exit()`:
///   - **On CI:** 1 × SIGINT_TICK (≈ 0.15s) — two graceful SIGINTs, then exit.
///   - **Off CI:** 2 × SIGINT_TICK (≈ 0.3s) + SIGINT_GRACE (12s) +
///     POST_KILL_GRACE (1s) ≈ 13.3s.
/// Both well under typical CI cancel grace periods; the long off-CI window is
/// just developer-machine headroom and never runs on a CI runner.

/// Watch for SIGINT / SIGTERM. On first signal we send bazel a SIGINT burst.
/// Bazel responds the same way it would to repeated Ctrl-Cs from a terminal
/// (see https://bazel.build/run/cancellation):
///
///   1st  →  graceful cancel of the running command.
///   2nd  →  still graceful (bazel allows a short cleanup window).
///   3rd  →  bazel calls `KillServerProcess` and hard-exits the client.
///
/// The burst, and what follows it, differ by environment:
///
///   - **On CI** we send only the 1st and 2nd SIGINTs — both graceful — and
///     then exit. We deliberately skip the 3rd SIGINT (which would trigger
///     `KillServerProcess`) and never self-SIGKILL. CI runners don't reap our
///     process tree on job cancellation (GHA's `cleanProcessTable` /
///     `KILL_PROCESSES` defaults off, and the runner systemd unit is
///     `KillMode=process`), so the only thing that would hard-kill bazel
///     mid-cleanup is *us* — and a `KillServerProcess` or SIGKILL landing
///     during `beforeCommand` sandbox setup is what strands a
///     `<output_base>/sandbox/linux-sandbox/…` tree on disk (the
///     bazelbuild/bazel#23880 wreckage) and poisons the next command. Leaving
///     bazel on the graceful-cancel path lets `afterCommand` finish cleanup on
///     its own clock; if a poisoned base survives anyway, the next job on this
///     runner hits the build-start health check (PR #1185) that detects and
///     repairs it.
///
///   - **Off CI** (interactive/local) there is no next-job health check and no
///     external reaper, so we keep the full escalation: all three SIGINTs,
///     then sleep `SIGINT_GRACE`, SIGKILL anything still alive, sleep
///     `POST_KILL_GRACE`. This matches a developer hammering Ctrl-C and
///     expecting bazel to actually die.
///
/// Then `std::process::exit(N)` — 130 for SIGINT, 143 for SIGTERM (the
/// "killed by signal N" shell convention is 128 + N).
///
/// **Why force-exit instead of letting Drop and unwind do their thing:**
/// the AXL drain loop runs on a `spawn_blocking` thread. Blocking
/// work in there (network calls in feature handlers, Starlark
/// evaluation, etc.) doesn't yield to the tokio scheduler, so there's
/// no clean way to ask it to stop cooperatively. Without force-exit,
/// a single hung handler could keep aspect-cli alive past
/// cancellation — which is exactly the CI hang this whole module is
/// guarding against.
///
/// **Relationship to AXL's own 3-SIGINT path** (`engine/bazel/cancel.rs`):
/// that one is invoked by AXL code via `ctx.bazel.cancel()` to cancel
/// a specific in-flight build cooperatively; this one is invoked by
/// the *operating system* signal to aspect-cli itself. They can fire
/// independently — if both happen, bazel just sees a flurry of SIGINTs,
/// which it handles per its own cancellation state machine.
///
/// Runs as a detached tokio task; never returns (either it terminates
/// the process or its host runtime dies first).
fn install_shutdown_handler() {
    #[cfg(unix)]
    tokio::spawn(async {
        use tokio::signal::unix::{SignalKind, signal};

        let mut sigint = match signal(SignalKind::interrupt()) {
            Ok(s) => s,
            Err(e) => {
                tracing::warn!("install_shutdown_handler: failed to install SIGINT handler: {e}");
                return;
            }
        };
        // If SIGTERM install fails, fall back to SIGINT-only — do NOT return.
        // Per tokio's docs, dropping a `Signal` stream does not uninstall the
        // OS-level handler, so returning here would leave SIGINT registered
        // with no listener: tokio would swallow Ctrl-C and aspect-cli would
        // appear unkillable except via an external SIGKILL — exactly the hang
        // this whole module exists to prevent.
        let mut sigterm = match signal(SignalKind::terminate()) {
            Ok(s) => Some(s),
            Err(e) => {
                tracing::warn!(
                    "install_shutdown_handler: failed to install SIGTERM handler ({e}); \
                     continuing with SIGINT-only shutdown"
                );
                None
            }
        };

        let signal_name = match sigterm.as_mut() {
            Some(sigterm) => tokio::select! {
                _ = sigint.recv()  => "SIGINT",
                _ = sigterm.recv() => "SIGTERM",
            },
            None => {
                sigint.recv().await;
                "SIGINT"
            }
        };
        let exit_code = if signal_name == "SIGINT" { 130 } else { 143 };

        run_shutdown_sequence(signal_name, exit_code).await;
    });

    #[cfg(not(unix))]
    {
        tokio::spawn(async {
            if tokio::signal::ctrl_c().await.is_ok() {
                run_shutdown_sequence("Ctrl+C", 130).await;
            }
        });
    }
}

async fn run_shutdown_sequence(signal_name: &str, exit_code: i32) {
    eprintln!("aspect-cli: received {signal_name}, cancelling bazel subprocesses…");

    // Two graceful SIGINTs; the CI/off-CI split below decides what follows.
    // See `install_shutdown_handler` for the full rationale.
    bazel_live::signal_all_for_shutdown();
    tokio::time::sleep(SIGINT_TICK).await;
    bazel_live::signal_all_for_shutdown();

    if on_recognized_ci() {
        // Stop short of KillServerProcess and SIGKILL — let bazel finish its
        // own cleanup so a cancellation can't strand a poisoned sandbox.
        eprintln!("aspect-cli: on CI, leaving bazel to wind down; exiting with code {exit_code}");
        std::process::exit(exit_code);
    }

    // Off CI: 3rd SIGINT (→ KillServerProcess), then SIGKILL the stragglers.
    tokio::time::sleep(SIGINT_TICK).await;
    bazel_live::signal_all_for_shutdown();

    tokio::time::sleep(SIGINT_GRACE).await;

    let killed = bazel_live::force_kill_all_remaining();
    if killed > 0 {
        eprintln!("aspect-cli: SIGKILL'd {killed} bazel subprocess(es) that didn't exit");
        tokio::time::sleep(POST_KILL_GRACE).await;
    }

    eprintln!("aspect-cli: exiting with code {exit_code}");
    std::process::exit(exit_code);
}
