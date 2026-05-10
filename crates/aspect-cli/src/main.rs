mod builtins;
mod cmd;
mod helpers;
mod trace;
mod trace_buffer;

use std::process::ExitCode;
use std::time::Duration;

use aspect_telemetry::{cargo_pkg_short_version, do_not_track, send_telemetry};
use axl_runtime::bazel_live;
use axl_runtime::eval::{Loader, ModuleEnv, MultiPhaseEval};
use axl_runtime::module::{AXL_ROOT_MODULE_NAME, Mod};
use axl_runtime::module::{DiskStore, ModEvaluator};
use tokio::task;
use tokio::task::spawn_blocking;
use tracing::info_span;

use crate::cmd::Cmd;
use crate::helpers::{find_repo_root, get_default_axl_search_paths, search_sources};

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
    // `bazel_live`, and force-exits aspect-cli after a grace period
    // so we don't outlive the cancellation. Without this, a CI cancel
    // can leave bazel clients orphaned on warm runners — they hold
    // the JVM-server lock and block every subsequent invocation.
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
    let repo_root = find_repo_root(&current_work_dir)
        .await
        .map_err(|_| anyhow::anyhow!("could not find root directory"))?;

    let disk_store = DiskStore::new(repo_root.clone());
    let mode = ModEvaluator::new(repo_root.clone());

    let root_mod = mode.evaluate(AXL_ROOT_MODULE_NAME.to_string(), repo_root.clone())?;
    let builtins = builtins::expand_builtins(repo_root.clone(), disk_store.builtins_path())?;
    let module_roots = disk_store.expand_store(&root_mod, builtins).await?;

    let mut modules: Vec<Mod> = vec![];
    for (name, root) in module_roots {
        let r#mod = mode.evaluate(name, root)?;
        axl_runtime::trace!("module @{} at {:?}", r#mod.name, r#mod.root);
        modules.push(r#mod)
    }

    let search_paths = get_default_axl_search_paths(&current_work_dir, &repo_root);
    let (scripts, configs) = search_sources(&search_paths).await?;

    // `_root` is entered on this thread; spawn_blocking moves work to a
    // different thread where the span stack is empty. Capture the span and
    // re-enter it on the worker so the phase spans nest under `root`.
    let parent_span = tracing::Span::current();
    let out = spawn_blocking(move || -> Result<ExitCode, anyhow::Error> {
        let _enter = parent_span.enter();
        let cli_version = cargo_pkg_short_version();

        ModuleEnv::with(|env| -> Result<ExitCode, anyhow::Error> {
            let loader = Loader::new(cli_version.clone(), repo_root.clone(), &modules);
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
                repo_root: &repo_root,
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

/// Time we wait for bazel clients to exit after the 3-SIGINT burst
/// before escalating to SIGKILL. 5s matches `FORCE_KILL_TIMEOUT_MS`
/// in `axl-runtime/src/engine/bazel/cancel.rs`, which is the timeout
/// AXL's own programmatic 3-SIGINT path uses to wait for the client
/// to exit before SIGKILL'ing. Reusing the same number here keeps
/// the two cancellation paths consistent.
const SIGINT_GRACE: Duration = Duration::from_secs(5);

/// Time we wait after SIGKILL for the kernel to deliver the signal
/// and the process accounting to settle before we exit. SIGKILL
/// can't be ignored, but on busy systems the actual termination
/// (and reaping by init) can lag a beat. A short final wait keeps
/// us from racing the kernel and exiting before children are gone.
const POST_KILL_GRACE: Duration = Duration::from_secs(1);

/// Total wall time between receiving the OS signal and `exit()`:
///   3 × SIGINT_TICK    (≈ 0.45s)  — emit the 3-SIGINT burst
///   + SIGINT_GRACE     (5s)        — wait for graceful exit
///   + POST_KILL_GRACE  (1s)        — let SIGKILL land if needed
///   ≈ 6.5s
/// Well under typical CI cancel grace periods (GHA gives ~7.5s
/// between SIGTERM and SIGKILL on cancel; Buildkite is configurable
/// but defaults higher), and well over what bazel needs for a clean
/// graceful cancel.

/// Watch for SIGINT / SIGTERM. On first signal:
///
///   1. Send SIGINT to every live bazel subprocess (registered in
///      `bazel_live`).
///   2. Sleep `SIGINT_TICK`, send 2nd SIGINT to the same set.
///   3. Sleep `SIGINT_TICK`, send 3rd SIGINT.
///
/// Bazel responds to those three SIGINTs the same way it would to
/// three Ctrl-Cs from a terminal:
///
///   1st  →  graceful cancel of the running command.
///   2nd  →  still graceful (bazel allows a short cleanup window).
///   3rd  →  bazel calls `KillServerProcess` and hard-exits the client.
///
/// (See https://bazel.build/run/cancellation)
///
/// We then sleep `SIGINT_GRACE` to let bazel's hard exit complete,
/// SIGKILL anything still alive, sleep `POST_KILL_GRACE`, and finally
/// `std::process::exit(N)` — 130 for SIGINT, 143 for SIGTERM (the
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
/// which it handles per its own cancellation state machine. The shared
/// `SIGINT_GRACE` constant keeps the timeouts aligned.
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
        let mut sigterm = match signal(SignalKind::terminate()) {
            Ok(s) => s,
            Err(e) => {
                tracing::warn!("install_shutdown_handler: failed to install SIGTERM handler: {e}");
                return;
            }
        };

        let signal_name = tokio::select! {
            _ = sigint.recv()  => "SIGINT",
            _ = sigterm.recv() => "SIGTERM",
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

    // 3-SIGINT burst — mirrors bazel's expected interactive cancel
    // sequence, escalating to KillServerProcess on the third SIGINT.
    bazel_live::signal_all_for_shutdown();
    tokio::time::sleep(SIGINT_TICK).await;
    bazel_live::signal_all_for_shutdown();
    tokio::time::sleep(SIGINT_TICK).await;
    bazel_live::signal_all_for_shutdown();

    // Wait for bazel clients to wind down on their own.
    tokio::time::sleep(SIGINT_GRACE).await;

    // Anything still alive after the grace window gets SIGKILL.
    let killed = bazel_live::force_kill_all_remaining();
    if killed > 0 {
        eprintln!("aspect-cli: SIGKILL'd {killed} bazel subprocess(es) that didn't exit");
        tokio::time::sleep(POST_KILL_GRACE).await;
    }

    eprintln!("aspect-cli: exiting with code {exit_code}");
    std::process::exit(exit_code);
}
