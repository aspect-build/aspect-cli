mod builtins;
mod cmd;
mod crash_handler;
mod credential_helper;
mod helpers;
mod trace;
mod trace_buffer;

/// Use mimalloc rather than the platform allocator.
///
/// The Linux release binaries are static-musl, whose mallocng is markedly
/// slower than mimalloc under the allocation patterns this CLI produces —
/// parsing tens of thousands of build events, and building large Starlark
/// heaps — and takes a single global lock across every thread. mimalloc keeps
/// per-thread free lists, so the BES/sink/probe threads stop contending with
/// the Starlark thread on every allocation.
///
/// Built in mimalloc's secure mode (`MI_SECURE=4`): guard pages around
/// metadata, encoded free lists, randomized placement, and double-free
/// detection. That restores a property the platform allocator gave us for free
/// — musl's mallocng validates a check byte on every allocation, so heap
/// corruption aborted the process by itself, whereas a default mimalloc build
/// performs no equivalent check and would let the same corruption pass
/// silently. Costs roughly 7% on an allocation-heavy workload, with no change
/// in binary size. See [`install_allocator_error_handler`], which makes any
/// detection fatal rather than merely reported.
#[global_allocator]
static GLOBAL: mimalloc::MiMalloc = mimalloc::MiMalloc;

/// Abort the process when mimalloc detects heap corruption.
///
/// mimalloc's built-in handling is not sufficient on its own. It aborts only
/// on `EFAULT` (corrupted metadata, corrupted thread-free list, and — in
/// secure mode — a detected buffer overflow), while a double free
/// (`EAGAIN`) or a free of an invalid pointer (`EINVAL`) is reported and then
/// execution *continues*. Continuing on a corrupted heap is what makes this
/// class of bug so hard to trace: the eventual crash lands somewhere
/// unrelated, long after the write that caused it.
///
/// Registering our own handler makes every corruption code fatal at the point
/// of detection. The resulting SIGABRT is caught by the crash handler, which
/// prints the signal and a resolvable address.
///
/// Note that detection is separately silent unless `show_errors` is on, so the
/// message explaining *what* was detected only appears when the process was
/// launched with `MIMALLOC_SHOW_ERRORS=1`. The abort happens either way.
fn install_allocator_error_handler() {
    /// Codes that indicate heap corruption rather than a benign condition
    /// (`ENOMEM`/`EOVERFLOW` are allocation failures, not corruption).
    extern "C" fn on_error(code: std::ffi::c_int, _arg: *mut std::ffi::c_void) {
        if matches!(code, libc::EFAULT | libc::EAGAIN | libc::EINVAL) {
            std::process::abort();
        }
    }
    // SAFETY: registering a global error callback; the callback only aborts.
    unsafe { libmimalloc_sys::mi_register_error(Some(on_error), std::ptr::null_mut()) };
}

use axl_runtime::{TaskExit, errln, outln};
use std::path::Path;
use std::process::ExitCode;
use std::time::Duration;

use aspect_telemetry::{
    cargo_pkg_display_version, cargo_pkg_short_version, do_not_track, send_telemetry,
};
use axl_runtime::ci::on_recognized_ci;
use axl_runtime::eval::{Loader, ModuleEnv, MultiPhaseEval};
use axl_runtime::module::{AXL_ROOT_MODULE_NAME, Mod};
use axl_runtime::module::{DiskStore, ModEvaluator};
use axl_runtime::{SignalKind, Signals};
use tokio::task;
use tokio::task::spawn_blocking;
use tracing::info_span;

use crate::cmd::Cmd;
use crate::helpers::{find_user_config, get_default_axl_search_paths, search_sources};
use axl_runtime::project_root::{find_aspect_root, find_bazel_root, find_git_root};

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
    // Watch for Ctrl+C / SIGTERM before anything else can spawn a child.
    // A signal cancels the run's root cancellation token: every child the
    // runtime spawned starts its stop sequence, and the AXL task ends at its
    // next blocking call with its post-task hooks, defers and bookend intact
    // (`axl_runtime::engine::cancellation`). The task only ever sees a token;
    // this task owns the escalation that keeps aspect-cli killable.
    install_signal_watcher();

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

    // A directory removed under the shell — a released `aspect worktree` slot,
    // most often — is otherwise reported as a bare "No such file or directory".
    let current_work_dir = std::env::current_dir().map_err(|err| {
        if err.kind() == std::io::ErrorKind::NotFound {
            anyhow::Error::from(err).context(
                "the current directory no longer exists — if it was a worktree that was released, \
                 `cd` back into the clone",
            )
        } else {
            anyhow::Error::from(err)
        }
    })?;
    // `Env` requires both roots; cwd is the last-resort fallback when no
    // marker file exists anywhere up the tree.
    let aspect_root =
        find_aspect_root(&current_work_dir).unwrap_or_else(|| current_work_dir.clone());
    let bazel_root = find_bazel_root(&current_work_dir).unwrap_or_else(|| current_work_dir.clone());
    let git_root = find_git_root(&current_work_dir);

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

    // User-global overrides run last among configs, scoped to their own
    // module so loads resolve within ~/.aspect. Skipped when the aspect
    // root is the home dir itself — the file is already in `configs`.
    let user_config = find_user_config(dirs::home_dir().as_deref())
        .await
        .filter(|path| !configs.contains(path))
        .map(|path| {
            let dir = path
                .parent()
                .expect("config path has a parent")
                .to_path_buf();
            (path, Mod::user_config_scope(dir))
        });

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
                &modules,
            );
            let mut mpe = MultiPhaseEval::new(env, &loader);

            // Phase 1: discover tasks and features.
            mpe.eval(&scripts, &root_mod, &modules)
                .map_err(anyhow::Error::from)?;

            // Phase 2: run config files.
            let config_entries: Vec<(&Path, &Mod)> = configs
                .iter()
                .map(|path| (path.as_path(), &root_mod))
                .chain(
                    user_config
                        .iter()
                        .map(|(path, r#mod)| (path.as_path(), r#mod)),
                )
                .collect();
            mpe.execute_configs(&config_entries)
                .map_err(|err| anyhow::Error::from(err).context(ConfigError))?;

            // Build the CLI surface from current eval state.
            let cmd = Cmd {
                tasks: mpe.tasks(),
                features: mpe.features(),
                aspect_root: &aspect_root,
                modules: &modules,
            };
            let mut root_cmd = cmd.build(&cli_version)?;
            // Finalize the surface (this is what injects `--help`) so routing
            // sees every flag Clap knows, then let the selected task collect the
            // flags Clap would reject — see `Cmd::route_unrecognized_flags`.
            // Nothing to route means argv comes back verbatim, so a task that
            // forwards nothing still gets Clap's parse error.
            root_cmd.build();
            let mut cmd_for_help = root_cmd.clone();
            let argv = cmd.route_unrecognized_flags(&root_cmd, std::env::args_os().collect());

            let matches = match root_cmd.try_get_matches_from_mut(argv) {
                Ok(m) => m,
                Err(err) => {
                    err.print().ok();
                    return Ok(ExitCode::from(err.exit_code() as u8));
                }
            };

            match matches.subcommand_name() {
                Some("version") => {
                    outln!("{}", cargo_pkg_display_version());
                    return Ok(ExitCode::SUCCESS);
                }
                Some("help") => {
                    cmd_for_help.print_help()?;
                    return Ok(ExitCode::SUCCESS);
                }
                Some("describe") => {
                    let task = matches
                        .subcommand_matches("describe")
                        .and_then(|m| m.get_one::<String>("task"))
                        .map(String::as_str);
                    return Ok(cmd.print_describe(&cli_version, task));
                }
                Some("feature") => {
                    let name = matches
                        .subcommand_matches("feature")
                        .and_then(|m| m.get_one::<String>("name"))
                        .map(String::as_str);
                    return Ok(cmd.print_feature_help(&cli_version, name));
                }
                _ => {}
            }

            let dispatch = cmd.dispatch(matches)?;

            // Print the "Running <task>" header before feature
            // implementations run so any diagnostic output from feature
            // initialization (auth WARNINGs, tip blocks, etc.) is
            // framed by the header.
            mpe.print_running_task_header(
                dispatch.task_id,
                &dispatch.task_name,
                dispatch.task_name_meaningful,
            )
            .map_err(anyhow::Error::from)?;

            // Phase 3: run enabled feature impls.
            mpe.execute_features_with_args(|f, h| dispatch.feature_args(f, h))
                .map_err(anyhow::Error::from)?;

            // Phase 3.5: install exporters from any registered via
            // `ctx.telemetry.exporters.add(...)`. Replays buffered spans
            // and logs to them before phase 4 starts emitting task traces.
            // No-op (and disables further OTel work for the rest of the run)
            // if no exporter was registered.
            let mut exporters = mpe.drain_exporters();
            // These tasks' stdout carries a protocol payload another program
            // parses, so a configured stdout telemetry sink would interleave
            // with it and corrupt it. Redirect such sinks to stderr for them;
            // every other configuration is untouched.
            if let Some(owner) = stdout_protocol_owner(&dispatch.task_kind) {
                use axl_runtime::engine::telemetry::{ExporterSpec, FileDestination};
                for spec in &mut exporters {
                    if let ExporterSpec::File(file) = spec {
                        if file.destination == FileDestination::Stdout {
                            errln!(
                                "warning: a stdout telemetry exporter is configured, but \
                                 {owner} — redirecting it to stderr."
                            );
                            file.destination = FileDestination::Stderr;
                        }
                    }
                }
            }
            tokio::runtime::Handle::current().block_on(trace::install_late_exporters(exporters))?;

            // Phase 4: execute the selected task.
            let exit = mpe
                .execute_tasks_with_args(
                    dispatch.task_id,
                    dispatch.task_name.clone(),
                    dispatch.task_name_meaningful,
                    dispatch.task_friendly_name.clone(),
                    dispatch.task_uuid.clone(),
                    dispatch.timing,
                    |t, h| dispatch.task_args(t, h),
                )
                .map_err(anyhow::Error::from)?;

            mpe.finish();
            Ok(ExitCode::from(exit.unwrap_or(0)))
        })
    });

    let result = match out.await {
        Ok(result) => result,
        Err(err) => panic!("{:?}", err),
    };
    // Children bound to the root may still be stopping (a terminated process
    // in its grace, a bazel client finishing its cancel). Give them the exit
    // window before the process goes away; past it, say so and leave.
    if !Signals::global().drain(exit_drain()).await {
        errln!(
            "aspect-cli: some child processes were still stopping after {}s",
            exit_drain().as_secs()
        );
    }
    drop(_root);
    drop(_tracing);
    result
}

/// For a task whose stdout is a machine-parsed protocol payload rather than
/// console output, a phrase naming what owns it — otherwise `None`.
fn stdout_protocol_owner(task_kind: &str) -> Option<&'static str> {
    match task_kind {
        "mcp" => Some("`aspect mcp` owns stdout for the MCP protocol"),
        "workspace-data" => {
            Some("`aspect setup workspace-data` owns stdout for Bazel's workspace status")
        }
        _ => None,
    }
}

/// Marks an error raised while running `config.axl` files. It happens before
/// any task's arguments are parsed, so no task can report it in the shape its
/// `--output` asks for; `main` does that instead.
#[derive(Debug)]
struct ConfigError;

impl std::fmt::Display for ConfigError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("a config.axl file failed")
    }
}

/// Whether `args` ask for `--output=json` (or `--output json`) ahead of any
/// `--`. Read from the raw arguments because a config error stops the CLI
/// before they are parsed.
fn wants_json_output(args: impl IntoIterator<Item = String>) -> bool {
    let args: Vec<String> = args.into_iter().take_while(|a| a != "--").collect();
    args.iter().enumerate().any(|(i, arg)| {
        arg == "--output=json"
            || (arg == "--output" && args.get(i + 1).is_some_and(|v| v == "json"))
    })
}

fn main() -> ExitCode {
    // Install first, before any other machinery, so fatal-signal reporting
    // covers everything after it (see the `crash_handler` module docs).
    crash_handler::install();
    install_allocator_error_handler();
    crash_handler::trigger_test_crash();

    // Intercept the Bazel credential helper (`aspect get`) before the async
    // runtime and workspace discovery so it stays fast (see `credential_helper`).
    if credential_helper::is_invocation() {
        return match credential_helper::run() {
            Ok(()) => ExitCode::SUCCESS,
            Err(err) => {
                errln!("error: {err:?}");
                ExitCode::FAILURE
            }
        };
    }

    match run() {
        Ok(code) => code,
        Err(err) => {
            // An exit raised from a feature or config impl never reaches the
            // task runner, so it is recognized here instead.
            if let Some(exit) = TaskExit::from_anyhow(&err) {
                // `{err:#}` walks the chain to the Starlark error, whose
                // rendering carries the traceback.
                exit.report(&format_args!("{err:#}"));
                return ExitCode::from(exit.code);
            }
            // A config error is reported as the failed config's own error,
            // and as a document on stdout when one was asked for: a caller
            // reading `--output=json` otherwise gets nothing to parse.
            if err.downcast_ref::<ConfigError>().is_some() {
                let message = format!("{:#}", err.root_cause());
                errln!("error: {}", message.trim_start_matches("error: "));
                if wants_json_output(std::env::args().skip(1)) {
                    outln!(
                        "{}",
                        serde_json::json!({
                            "schema_version": 1,
                            "error": "config_error",
                            "message": message.trim_start_matches("error: "),
                        })
                    );
                }
                return ExitCode::FAILURE;
            }
            errln!("error: {err:?}");
            ExitCode::FAILURE
        }
    }
}

/// How long, once the task has ended or the second signal has arrived, the
/// children still stopping get before the process exits regardless. On CI it
/// must fit inside the host's own cancel window: the Buildkite agent SIGKILLs
/// 9s after its SIGTERM and GitHub Actions escalates 7.5s after its SIGINT. Off
/// CI it covers a terminated child's grace (`CHILD_GRACE`, 3s) plus a beat for
/// the kernel to settle.
fn exit_drain() -> Duration {
    if on_recognized_ci() {
        Duration::from_secs(5)
    } else {
        Duration::from_secs(4)
    }
}

/// Record every Ctrl+C / SIGTERM into the run's cancellation state, and own
/// the escalation that no AXL code can defeat.
///
/// The first signal only cancels the root token (or the token a task took
/// with `ctx.cancellation.notify()`); the task unwinds cooperatively and
/// `run()` waits for the children's stop sequences. This task then waits for
/// one of two things:
///
///   - **a second signal**, a human hammering Ctrl+C or a host escalating:
///     kill every child still alive and exit at once with 128 + the signal;
///   - **the exit window passing after a terminate**: same, so a hung task
///     cannot keep aspect-cli alive past a CI cancel.
///
/// On CI the bazel client is spared the kill either way (the host's own
/// escalation is the last rung there): a SIGKILL landing during sandbox
/// cleanup strands `_moved_trash_dir` and poisons the next command on the
/// runner (bazelbuild/bazel#23880).
///
/// Runs as a detached tokio task for the life of the process. Per tokio's
/// docs, dropping a `Signal` stream does not uninstall the OS handler, so it
/// never returns early: returning would leave the signal registered with no
/// listener, and aspect-cli unkillable except by SIGKILL.
fn install_signal_watcher() {
    tokio::spawn(async {
        let Some(first) = next_signal().await else {
            return;
        };
        let signals = Signals::global();
        signals.record(first);
        errln!("aspect-cli: {}ed, stopping…", first.name());

        let second = async {
            let kind = next_signal().await.unwrap_or(first);
            signals.record(kind);
            kind
        };
        let kind = match first {
            SignalKind::Interrupt => second.await,
            SignalKind::Terminate => tokio::select! {
                kind = second => kind,
                _ = tokio::time::sleep(exit_drain()) => first,
            },
        };
        let killed = signals.kill_all(on_recognized_ci());
        if killed > 0 {
            errln!("aspect-cli: killed {killed} child process(es) that were still running");
        }
        errln!("aspect-cli: exiting with code {}", kind.exit_code());
        std::process::exit(i32::from(kind.exit_code()));
    });
}

/// The next OS request to stop, as the kind the runtime understands.
/// `None` only if no handler could be installed at all.
#[cfg(unix)]
async fn next_signal() -> Option<SignalKind> {
    use std::sync::OnceLock;
    use tokio::signal::unix::{Signal, SignalKind as Unix, signal};
    use tokio::sync::Mutex;

    // Installed once and kept for the life of the process (see above).
    static STREAMS: OnceLock<Mutex<(Option<Signal>, Option<Signal>)>> = OnceLock::new();
    let streams = STREAMS.get_or_init(|| {
        let interrupt = signal(Unix::interrupt())
            .map_err(|e| tracing::warn!("failed to install the SIGINT handler: {e}"))
            .ok();
        // If SIGTERM cannot be installed, carry on with SIGINT alone.
        let terminate = signal(Unix::terminate())
            .map_err(|e| tracing::warn!("failed to install the SIGTERM handler: {e}"))
            .ok();
        Mutex::new((interrupt, terminate))
    });
    let mut guard = streams.lock().await;
    let (interrupt, terminate) = &mut *guard;
    match (interrupt.as_mut(), terminate.as_mut()) {
        (Some(int), Some(term)) => Some(tokio::select! {
            _ = int.recv() => SignalKind::Interrupt,
            _ = term.recv() => SignalKind::Terminate,
        }),
        (Some(int), None) => {
            int.recv().await;
            Some(SignalKind::Interrupt)
        }
        (None, Some(term)) => {
            term.recv().await;
            Some(SignalKind::Terminate)
        }
        (None, None) => None,
    }
}

#[cfg(windows)]
async fn next_signal() -> Option<SignalKind> {
    use std::sync::OnceLock;
    use tokio::signal::windows::{
        CtrlBreak, CtrlC, CtrlClose, CtrlLogoff, CtrlShutdown, ctrl_break, ctrl_c, ctrl_close,
        ctrl_logoff, ctrl_shutdown,
    };
    use tokio::sync::Mutex;

    type Streams = (
        Option<CtrlC>,
        Option<CtrlBreak>,
        Option<CtrlClose>,
        Option<CtrlLogoff>,
        Option<CtrlShutdown>,
    );
    static STREAMS: OnceLock<Mutex<Streams>> = OnceLock::new();
    let streams = STREAMS.get_or_init(|| {
        Mutex::new((
            ctrl_c().ok(),
            ctrl_break().ok(),
            ctrl_close().ok(),
            ctrl_logoff().ok(),
            ctrl_shutdown().ok(),
        ))
    });
    let mut guard = streams.lock().await;
    let (c, brk, close, logoff, shutdown) = &mut *guard;
    if c.is_none() && brk.is_none() && close.is_none() && logoff.is_none() && shutdown.is_none() {
        return None;
    }
    Some(tokio::select! {
        _ = async { match c.as_mut() { Some(s) => s.recv().await, None => std::future::pending().await } } => SignalKind::Interrupt,
        _ = async { match brk.as_mut() { Some(s) => s.recv().await, None => std::future::pending().await } } => SignalKind::Interrupt,
        _ = async { match close.as_mut() { Some(s) => s.recv().await, None => std::future::pending().await } } => SignalKind::Terminate,
        _ = async { match logoff.as_mut() { Some(s) => s.recv().await, None => std::future::pending().await } } => SignalKind::Terminate,
        _ = async { match shutdown.as_mut() { Some(s) => s.recv().await, None => std::future::pending().await } } => SignalKind::Terminate,
    })
}

#[cfg(not(any(unix, windows)))]
async fn next_signal() -> Option<SignalKind> {
    tokio::signal::ctrl_c()
        .await
        .ok()
        .map(|_| SignalKind::Interrupt)
}

#[cfg(test)]
mod print_macro_guard {
    /// `println!` and friends panic on a failed write, which strands a task
    /// mid-run when a pipeline reader leaves. See docs/axl.md.
    #[test]
    fn no_panicking_print_macros() {
        static SRC: include_dir::Dir<'_> = include_dir::include_dir!("$CARGO_MANIFEST_DIR/src");
        let found = axl_runtime::out::panicking_print_macros(&SRC);
        assert!(
            found.is_empty(),
            "use outln!/errln!/out! from axl_runtime::out instead (see docs/axl.md):\n  {}",
            found.join("\n  ")
        );
    }
}

#[cfg(test)]
mod config_error_tests {
    use super::wants_json_output;

    fn args(list: &[&str]) -> Vec<String> {
        list.iter().map(|a| a.to_string()).collect()
    }

    #[test]
    fn json_output_is_recognised_in_both_spellings() {
        assert!(wants_json_output(args(&["gc", "--output=json"])));
        assert!(wants_json_output(args(&["gc", "--output", "json"])));
        assert!(!wants_json_output(args(&["gc", "--output=text"])));
        assert!(!wants_json_output(args(&["gc", "--output"])));
    }

    #[test]
    fn arguments_after_a_double_dash_are_not_ours() {
        assert!(!wants_json_output(args(&["build", "--", "--output=json"])));
    }
}
