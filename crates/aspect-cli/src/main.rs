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
use axl_runtime::eval::{AxlFileFailure, AxlFileRole, Loader, ModuleEnv, MultiPhaseEval};
use axl_runtime::module::{AXL_ROOT_MODULE_NAME, Mod};
use axl_runtime::module::{DiskStore, ModEvaluator};
use axl_runtime::{SignalKind, SignalOrigin, Signals};
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

            // What the two phases could not load, and what that costs the
            // command the user typed — see `Unloadable` for the policy it
            // implements. Past the degraded-run decision below there is
            // nothing left unreported, so the raises after it are ordinary.
            let mut unloadable = Unloadable::default();

            // Phase 1: discover tasks and features.
            let phase1 = mpe.eval(&scripts, &root_mod, &modules);
            unloadable.scripts = phase1.files;
            if let Some(error) = phase1.ended {
                return Err(unloadable.reporting(error.into()));
            }

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
            let phase2 = mpe.execute_configs(&config_entries);
            unloadable.configs = phase2.files;
            if let Some(error) = phase2.ended {
                return Err(unloadable
                    .reporting(anyhow::Error::from(error))
                    .context(ConfigError));
            }

            // Build the CLI surface from current eval state.
            let cmd = Cmd {
                tasks: mpe.tasks(),
                features: mpe.features(),
                aspect_root: &aspect_root,
                modules: &modules,
            };
            // A name conflict or a reserved name is judged here, on the
            // finished surface, so it is one of the raise paths that has to
            // report what did not load on its way out.
            let mut root_cmd = cmd
                .build(&cli_version)
                .map_err(|err| unloadable.reporting(err.into()))?;
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
                    // `--help`, `--version` and a bare `aspect` all land here:
                    // Clap renders them itself rather than failing, and none
                    // needs a task definition, so they still run degraded.
                    if is_display_only(&err) {
                        unloadable.warn();
                        err.print().ok();
                        return Ok(ExitCode::from(err.exit_code() as u8));
                    }
                    // Any other parse error was judged against a surface built
                    // from a workspace that did not fully load, so the command
                    // the user typed may be one of the tasks that went missing.
                    // The load failure explains that; Clap's does not.
                    if let Some(err) = unloadable.into_error() {
                        return Err(err);
                    }
                    err.print().ok();
                    return Ok(ExitCode::from(err.exit_code() as u8));
                }
            };

            // Everything past here either reports the CLI's own surface or
            // dispatches a task; see `Unloadable` for which may run degraded.
            if !unloadable.is_empty() {
                if runs_without_tasks(matches.subcommand_name()) {
                    unloadable.warn();
                } else if let Some(err) = unloadable.into_error() {
                    return Err(err);
                }
            }

            match matches.subcommand_name() {
                Some(cmd::VERSION_COMMAND) => {
                    outln!("{}", cargo_pkg_display_version());
                    return Ok(ExitCode::SUCCESS);
                }
                Some(cmd::HELP_COMMAND) => {
                    cmd_for_help.print_help()?;
                    return Ok(ExitCode::SUCCESS);
                }
                Some(cmd::DESCRIBE_COMMAND) => {
                    let task = matches
                        .subcommand_matches(cmd::DESCRIBE_COMMAND)
                        .and_then(|m| m.get_one::<String>("task"))
                        .map(String::as_str);
                    return Ok(cmd.print_describe(&cli_version, task));
                }
                Some(cmd::FEATURE_COMMAND) => {
                    let name = matches
                        .subcommand_matches(cmd::FEATURE_COMMAND)
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

/// AXL that failed during startup, held rather than raised.
///
/// The whole CLI surface — the task list, their flags, the features — is
/// defined in AXL, so one `.aspect/*.axl` file that does not parse used to
/// fail every command: the `aspect help` the user needs to find the typo and
/// the `aspect version` they run to check their setup included. Phases 1 and 2
/// carry on past an unloadable file ([`MultiPhaseEval::eval`],
/// [`MultiPhaseEval::execute_configs`]) and hand what failed to this type.
///
/// `docs/design.md`, "When a file does not load", states the policy: which
/// commands run degraded, what each failure is reported as, and the four
/// things settled outside the tolerant phases that still fail everything.
/// This type is the mechanism, and holds two invariants the policy depends on:
///
/// * Nothing recorded is ever dropped silently. Both ways out of here report
///   first — [`Self::reporting`] for an error of its own, [`Self::into_error`]
///   for one of the recorded failures raised as the run's error — so a typo
///   does not vanish behind whatever ended the run.
/// * Scripts come before configs, and a config that failed alongside a script
///   is summarised as derived from it rather than given a warning of its own.
#[derive(Default)]
struct Unloadable {
    /// Phase 1: task and feature scripts.
    scripts: Vec<AxlFileFailure>,
    /// Phase 2: `config.axl` files.
    configs: Vec<AxlFileFailure>,
}

impl Unloadable {
    fn is_empty(&self) -> bool {
        self.scripts.is_empty() && self.configs.is_empty()
    }

    /// `err`, after reporting everything recorded. The one way to raise an
    /// error of its own out of startup: holding the two together is what makes
    /// "every raise path reports first" a single place rather than a rule each
    /// `return Err` has to remember.
    fn reporting(&self, err: anyhow::Error) -> anyhow::Error {
        self.warn();
        err
    }

    /// Report the failures on stderr for a run that carries on: a `warning:`
    /// line per script naming the file and what its absence costs, and any
    /// config that failed alongside them summarised rather than warned about
    /// separately. This is [`Self::warn_blaming`] with no raised failure, so
    /// the summary blames a script only when exactly one of them failed.
    ///
    /// Silence here would be worse than the old hard failure — the point is
    /// that the user still sees the typo.
    fn warn(&self) {
        self.warn_blaming(None);
    }

    /// [`Self::warn`], plus the script failure [`Self::into_error`] has taken
    /// off `scripts` to raise as the run’s error.
    ///
    /// Which script the derived-config summary blames: `raised` if there is
    /// one, it being off the list but still the likeliest cause; otherwise the
    /// only script that failed, if exactly one did. With several script
    /// failures and none raised, the summary points at them as a group —
    /// naming one of them would be repeating whatever order `read_dir`
    /// happened to use. A named cause is preferred to "the above" because a
    /// raised failure is not above: it prints below as the run’s `error:`.
    /// `ASPECT_DEBUG` prints the suppressed config diagnostic anyway, for the
    /// case where the guess is wrong and the config really did have its own
    /// problem.
    fn warn_blaming(&self, raised: Option<&AxlFileFailure>) {
        for failure in &self.scripts {
            Self::warn_one(failure);
        }

        // Nothing failed that a config could be derived from, so each config
        // owns its failure.
        if raised.is_none() && self.scripts.is_empty() {
            for failure in &self.configs {
                Self::warn_one(failure);
            }
            return;
        }
        if self.configs.is_empty() {
            return;
        }
        let cause = raised.or(match self.scripts.as_slice() {
            [only] => Some(only),
            _ => None,
        });
        let blame = match cause {
            Some(failure) => format!("{} — fix that first", failure.path.display()),
            None => "the failures above — fix those first".to_owned(),
        };
        let paths: Vec<String> = self
            .configs
            .iter()
            .map(|f| f.path.display().to_string())
            .collect();
        errln!(
            "warning: {} then failed too, most likely because of {blame}",
            paths.join(", "),
        );
        for failure in &self.configs {
            TaskExit::debug_traceback(&format_args!(
                "{}",
                strip_error_prefix(&failure.error.to_string())
            ));
        }
    }

    /// One `warning:` line naming the file and what its failure costs,
    /// followed by the file's own diagnostic.
    fn warn_one(failure: &AxlFileFailure) {
        errln!(
            "warning: {} failed; {}",
            failure.path.display(),
            cost_of_losing(failure.role)
        );
        errln!("{}", strip_error_prefix(&failure.error.to_string()));
    }

    /// The run's failure, for a command that cannot proceed without the AXL
    /// that did not load. Scripts are raised ahead of configs, a config error
    /// typically being a consequence of a task that never loaded. A phase-2
    /// failure keeps the [`ConfigError`] marker so `main` still renders the
    /// `--output=json` document for it.
    ///
    /// Everything *else* recorded is reported on the way out, so a second
    /// broken file is not lost to whichever one `read_dir` happened to hand
    /// over first. The raised failure is left out of that reporting: `main`
    /// prints it as the run's `error:`, and warning about it as well would
    /// print the same diagnostic twice.
    fn into_error(&mut self) -> Option<anyhow::Error> {
        if !self.scripts.is_empty() {
            let failure = self.scripts.remove(0);
            self.warn_blaming(Some(&failure));
            return Some(anyhow::Error::from(failure.error));
        }
        if self.configs.is_empty() {
            return None;
        }
        let failure = self.configs.remove(0);
        self.warn_blaming(None);
        Some(anyhow::Error::from(failure.error).context(ConfigError))
    }
}

/// What the user loses with an AXL file that did not load, as the clause its
/// `warning:` line ends with. Derived from the role the phase was loading it
/// for: a `use_feature` file defines no tasks, and the flags its feature adds
/// to every task subcommand go missing with it.
///
/// The clause says only what was lost. How the file broke is not something the
/// role knows — a `use_task` naming a missing symbol fails a file that parses
/// perfectly — so [`Unloadable::warn_one`] supplies the verb.
fn cost_of_losing(role: AxlFileRole) -> &'static str {
    match role {
        AxlFileRole::Tasks => "the tasks it defines are unavailable",
        AxlFileRole::Feature => {
            "the feature it defines is unavailable, along with the flags it adds to every task"
        }
        AxlFileRole::TasksAndFeature => {
            "the tasks and the feature it defines are unavailable, along with the flags that \
             feature adds to every task"
        }
        AxlFileRole::Config => "the settings it applies are missing or incomplete",
    }
}

/// Whether `subcommand` reports the CLI's own identity rather than anything
/// AXL defines, and so may run against a workspace that did not fully load.
fn runs_without_tasks(subcommand: Option<&str>) -> bool {
    subcommand.is_some_and(|name| cmd::IDENTITY_COMMANDS.contains(&name))
}

/// Whether Clap stopped to render help or the version rather than to reject
/// the command line. `DisplayHelpOnMissingArgumentOrSubcommand` covers both a
/// bare `aspect` and `aspect <group>` with no leaf, each of which prints that
/// level's help under `arg_required_else_help`.
fn is_display_only(err: &clap::Error) -> bool {
    use clap::error::ErrorKind;
    matches!(
        err.kind(),
        ErrorKind::DisplayHelp
            | ErrorKind::DisplayVersion
            | ErrorKind::DisplayHelpOnMissingArgumentOrSubcommand
    )
}

/// Drop the `error: ` a Starlark diagnostic renders for itself, so a caller
/// can put the message behind exactly one prefix of its own. Without this the
/// CLI printed `error: error: Parse error: …` for every AXL failure.
///
/// Every leading repetition goes, not just one: each layer that rendered the
/// error added a prefix, and the number of them is a property of how deeply
/// the phases wrapped it rather than anything a caller can predict. Stripping
/// once would leave the doubled form this exists to fix.
fn strip_error_prefix(message: &str) -> &str {
    message.trim_start_matches("error: ")
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
                errln!("error: {}", strip_error_prefix(&format!("{err:?}")));
                ExitCode::FAILURE
            }
        };
    }

    match run() {
        Ok(code) => code,
        Err(err) => {
            // An exit raised from a feature or config impl never reaches the
            // task runner, so it is recognized here instead.
            if let Some(exit) = TaskExit::from_anyhow(&err, Signals::global()) {
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
                errln!("error: {}", strip_error_prefix(&message));
                if wants_json_output(std::env::args().skip(1)) {
                    outln!(
                        "{}",
                        serde_json::json!({
                            "schema_version": 1,
                            "error": "config_error",
                            "message": strip_error_prefix(&message),
                        })
                    );
                }
                return ExitCode::FAILURE;
            }
            errln!("error: {}", strip_error_prefix(&format!("{err:?}")));
            ExitCode::FAILURE
        }
    }
}

/// How long the AXL body gets, after a signal, to return on its own before
/// it is ended. Long enough for an interrupted bazel to exit and its events
/// to drain; short enough, with the exit window after it, to fit the CI cancel
/// windows (the Buildkite agent SIGKILLs 9s after its SIGTERM, GitHub Actions
/// escalates 7.5s after its SIGINT).
fn body_grace() -> Duration {
    if on_recognized_ci() {
        Duration::from_secs(3)
    } else {
        Duration::from_secs(5)
    }
}

/// The body's grace when nothing was bound to the root at the signal: a task
/// at a prompt or in a pure computation has nothing to wait for.
const PROMPT_GRACE: Duration = Duration::from_secs(1);

/// How long, once the task has ended or the second signal has arrived, the
/// children still stopping get before the process exits regardless.
fn exit_drain() -> Duration {
    Duration::from_secs(4)
}

/// Record every Ctrl+C / SIGTERM into the run's cancellation state, and own
/// the escalation that no AXL code can defeat.
///
/// The first signal only cancels the root token: every child bound to it
/// starts stopping, and the AXL body is left to end on its own, its waits
/// returning as the children go. This task then gives the body a deadline:
/// [`body_grace`], or [`PROMPT_GRACE`] when nothing was bound to the root at
/// the signal (a prompt, a pure computation), since there is nothing to wait
/// for. Past it the body is ended at its next blocking call with 128 + the
/// signal, hooks and defers still running. Then, or on a second signal at
/// any point, the backstop: kill every child still alive and exit at once.
///
/// On CI the bazel client is spared the kill (the host's own escalation is
/// the last rung there): a SIGKILL landing during sandbox cleanup strands
/// `_moved_trash_dir` and poisons the next command on the runner
/// (bazelbuild/bazel#23880).
///
/// Runs as a detached tokio task for the life of the process. Per tokio's
/// docs, dropping a `Signal` stream does not uninstall the OS handler, so it
/// never returns early: returning would leave the signal registered with no
/// listener, and aspect-cli unkillable except by SIGKILL.
fn install_signal_watcher() {
    tokio::spawn(async {
        let Some((first, origin)) = next_signal().await else {
            return;
        };
        let signals = Signals::global();
        signals.record(first, origin);
        errln!("aspect-cli: {}ed, stopping…", first.name());

        let grace = if signals.has_live_children() {
            body_grace()
        } else {
            PROMPT_GRACE
        };
        let kind = tokio::select! {
            kind = another_signal(first) => kind,
            _ = tokio::time::sleep(grace) => {
                // The body did not return on its own: end it, then give the
                // exit path (hooks, defers, draining) the exit window before
                // the backstop.
                signals.force();
                tokio::select! {
                    kind = another_signal(first) => kind,
                    _ = tokio::time::sleep(exit_drain()) => first,
                }
            }
        };
        let killed = signals.kill_all(on_recognized_ci());
        if killed > 0 {
            errln!("aspect-cli: killed {killed} child process(es) that were still running");
        }
        errln!("aspect-cli: exiting with code {}", kind.exit_code());
        std::process::exit(i32::from(kind.exit_code()));
    });
}

/// A repeat of the request: recorded, and the backstop's cue.
async fn another_signal(first: SignalKind) -> SignalKind {
    let (kind, origin) = next_signal()
        .await
        .unwrap_or((first, SignalOrigin::Process));
    Signals::global().record(kind, origin);
    kind
}

/// The next OS request to stop, as the kind the runtime understands, and
/// where it came from. `None` only if no handler could be installed at all.
///
/// The origin matters because a terminal delivers Ctrl+C to the whole
/// foreground process group, children included, while `kill(2)` from a CI
/// runner reaches this process alone. tokio's streams do not expose the
/// `siginfo`, so a sigaction hook registered beside them (signal-hook chains
/// the handlers) records each signal's `si_pid`: a sending process fills it
/// in, the kernel leaves it zero, which for these two signals means the
/// terminal. (`si_code` would be the textbook field, but macOS reports 0 for
/// `kill(2)` too, so it cannot tell the two apart.)
#[cfg(unix)]
async fn next_signal() -> Option<(SignalKind, SignalOrigin)> {
    use std::sync::OnceLock;
    use std::sync::atomic::{AtomicI32, Ordering};
    use tokio::signal::unix::{Signal, SignalKind as Unix, signal};
    use tokio::sync::Mutex;

    /// The pid of the last signal's sender; 0 when the kernel sent it.
    static LAST_SENDER: AtomicI32 = AtomicI32::new(0);

    fn sender_of(info: &libc::siginfo_t) -> i32 {
        // Linux keeps the sender behind an accessor into the siginfo union,
        // which is zero for a kernel-generated signal; the BSDs expose it as
        // a field.
        #[cfg(any(target_os = "linux", target_os = "android"))]
        // SAFETY: `si_pid` is valid to read for SIGINT and SIGTERM, whose
        // siginfo carries the kill layout (or is zeroed by the kernel).
        unsafe {
            info.si_pid()
        }
        #[cfg(not(any(target_os = "linux", target_os = "android")))]
        info.si_pid
    }

    // Installed once and kept for the life of the process (see above).
    static STREAMS: OnceLock<Mutex<(Option<Signal>, Option<Signal>)>> = OnceLock::new();
    let streams = STREAMS.get_or_init(|| {
        for signo in [libc::SIGINT, libc::SIGTERM] {
            // SAFETY: the action only stores an integer into an atomic, which
            // is async-signal-safe.
            let registered = unsafe {
                signal_hook_registry::register_sigaction(signo, |info: &libc::siginfo_t| {
                    LAST_SENDER.store(sender_of(info), Ordering::SeqCst);
                })
            };
            if let Err(e) = registered {
                tracing::warn!("failed to install the signal-origin hook for {signo}: {e}");
            }
        }
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
    let kind = match (interrupt.as_mut(), terminate.as_mut()) {
        (Some(int), Some(term)) => tokio::select! {
            _ = int.recv() => SignalKind::Interrupt,
            _ = term.recv() => SignalKind::Terminate,
        },
        (Some(int), None) => {
            int.recv().await;
            SignalKind::Interrupt
        }
        (None, Some(term)) => {
            term.recv().await;
            SignalKind::Terminate
        }
        (None, None) => return None,
    };
    let origin = if LAST_SENDER.load(Ordering::SeqCst) != 0 {
        SignalOrigin::Process
    } else {
        SignalOrigin::Terminal
    };
    Some((kind, origin))
}

/// Console events never reach a child spawned with `CREATE_NEW_PROCESS_GROUP`,
/// so on Windows the runtime is always the only one signalling children.
#[cfg(windows)]
async fn next_signal() -> Option<(SignalKind, SignalOrigin)> {
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
    let kind = tokio::select! {
        _ = async { match c.as_mut() { Some(s) => s.recv().await, None => std::future::pending().await } } => SignalKind::Interrupt,
        _ = async { match brk.as_mut() { Some(s) => s.recv().await, None => std::future::pending().await } } => SignalKind::Interrupt,
        _ = async { match close.as_mut() { Some(s) => s.recv().await, None => std::future::pending().await } } => SignalKind::Terminate,
        _ = async { match logoff.as_mut() { Some(s) => s.recv().await, None => std::future::pending().await } } => SignalKind::Terminate,
        _ = async { match shutdown.as_mut() { Some(s) => s.recv().await, None => std::future::pending().await } } => SignalKind::Terminate,
    };
    Some((kind, SignalOrigin::Process))
}

#[cfg(not(any(unix, windows)))]
async fn next_signal() -> Option<(SignalKind, SignalOrigin)> {
    tokio::signal::ctrl_c()
        .await
        .ok()
        .map(|_| (SignalKind::Interrupt, SignalOrigin::Process))
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

#[cfg(test)]
mod degrade_policy_tests {
    use super::{is_display_only, runs_without_tasks, strip_error_prefix};
    use clap::error::ErrorKind;

    /// Starlark's own `error: ` title is dropped so the caller supplies the
    /// only one; anything else is left exactly as it came.
    #[test]
    fn only_leading_error_prefixes_are_dropped() {
        assert_eq!(
            strip_error_prefix("error: Parse error: x"),
            "Parse error: x"
        );
        assert_eq!(strip_error_prefix("Parse error: x"), "Parse error: x");
        assert_eq!(strip_error_prefix(""), "");
        // The shape the CLI actually printed before this existed: two layers
        // each rendered their own prefix, so both have to go.
        assert_eq!(
            strip_error_prefix("error: error: Parse error: x"),
            "Parse error: x"
        );
        // Not at the start, so not ours to touch.
        assert_eq!(
            strip_error_prefix("Traceback:\n  error: inner"),
            "Traceback:\n  error: inner"
        );
    }

    /// The commands that report the CLI's own identity, and nothing else.
    #[test]
    fn only_version_and_help_run_without_tasks() {
        assert!(runs_without_tasks(Some("version")));
        assert!(runs_without_tasks(Some("help")));
        for other in ["build", "test", "describe", "feature", "gc"] {
            assert!(!runs_without_tasks(Some(other)), "{other} must not degrade");
        }
        assert!(!runs_without_tasks(None));
    }

    /// Clap stopping to render help or a version is not Clap rejecting the
    /// command line, and only the former may run against a workspace that did
    /// not fully load.
    #[test]
    fn display_only_covers_help_version_and_a_bare_invocation() {
        for kind in [
            ErrorKind::DisplayHelp,
            ErrorKind::DisplayVersion,
            ErrorKind::DisplayHelpOnMissingArgumentOrSubcommand,
        ] {
            assert!(
                is_display_only(&clap::Error::raw(kind, "x")),
                "{kind:?} renders help rather than failing"
            );
        }
        for kind in [
            ErrorKind::InvalidSubcommand,
            ErrorKind::UnknownArgument,
            ErrorKind::MissingRequiredArgument,
            ErrorKind::InvalidValue,
        ] {
            assert!(
                !is_display_only(&clap::Error::raw(kind, "x")),
                "{kind:?} is a rejection, not a help screen"
            );
        }
    }
}
