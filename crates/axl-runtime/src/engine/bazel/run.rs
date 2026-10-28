//! `ctx.bazel.run` and `ctx.bazel.run_all`: build targets for running, the
//! way `bazel run` does, without changing a single configuration option.
//!
//! Both start one `bazel build` and inject only command options: the
//! `runinfo` aspect (what `bazel run` reads but no provider exposes: the
//! executable, the expanded `args` attribute, `RunEnvironmentInfo`), its
//! output group, and `--remote_download_outputs=toplevel` when the rc resolves
//! to `minimal`, which is the download set Bazel's own RUN mode computes. The
//! runfiles tree is built from the manifest after the fact ([`super::runfiles`]),
//! so `--build_runfile_links` is never touched and the analysis cache survives.
//!
//! A tracker thread follows the build's BES stream and settles one slot per
//! target the moment its default and `aspect_runinfo` output groups, and every
//! fileset they reference, have arrived. `run_all` hands those slots out as
//! `Future[Runnable]` in completion order; `run` additionally spawns the one
//! target's process as soon as it is ready and hands out `Future[Child]`.

use std::cell::{Cell, RefCell};
use std::collections::{HashMap, VecDeque};
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, mpsc};

use allocative::Allocative;
use derive_more::Display;
use futures::FutureExt;
use futures::future::{BoxFuture, Shared};
use starlark::environment::{Methods, MethodsBuilder, MethodsStatic};
use starlark::eval::Evaluator;
use starlark::values::dict::UnpackDictEntries;
use starlark::values::list::UnpackList;
use starlark::values::none::NoneOr;
use starlark::values::{
    self, AllocValue, Heap, NoSerialize, ProvidesStaticType, StarlarkValue, Trace, ValueLike,
    starlark_value,
};
use starlark::{StarlarkResultExt, starlark_module, starlark_simple_value};
use tokio::sync::oneshot;
use tokio_util::sync::CancellationToken;

use super::build::{Build, BuildEventIter, BuildStatus};
use super::wrappers::ExecLogSink;
use crate::engine::r#async::future::{FutureAlloc, FutureOf};
use crate::engine::cancellation::{self, Signals};
use crate::engine::children;
use crate::engine::error::native_error;
use crate::engine::std::process::{Child, Command, stdio_from};
use crate::engine::std::stream::Writable;
use crate::engine::store::Env;
use bazel::command::run::{Dirs, Failure, OnSettled, Outcome, PrepareError, Tracker};
use bazel::label::apparent_label;
use bazel::runfiles::SymlinksMode;

native_error! {
    /// A target cannot be run as asked, known before the build starts: the rc
    /// disables `--build_runfile_manifests`, which `bazel run` refuses too.
    pub(crate) struct RunError = "bazel.run.Error" {}
}

native_error! {
    /// The build that was to produce the target failed. `code` is Bazel's
    /// exit code; its diagnostics have already reached the terminal.
    pub(crate) struct BuildError = "bazel.run.BuildError" {
        /// The target, as it was asked for.
        target: String,
        /// Bazel's exit code for the build.
        code: i32,
    }
}

native_error! {
    /// The build ended without reporting this target built: an alias, a
    /// target filtered out, or one the build never reached after another
    /// target's failure stopped it.
    pub(crate) struct NotBuilt = "bazel.run.NotBuilt" {
        /// The target, as it was asked for.
        target: String,
    }
}

native_error! {
    /// The target built but has nothing to run: no executable among its
    /// default outputs, the shape of a library or a filegroup.
    pub(crate) struct NotRunnable = "bazel.run.NotRunnable" {
        /// The target, as it was asked for.
        target: String,
    }
}

/// The error a script sees for a slot that settled without a `Runnable`.
fn failure_error(failure: Failure) -> anyhow::Error {
    match failure {
        Failure::NotBuilt { target } => NotBuilt {
            message: format!("{target} was not built: the build ended without reporting it"),
            target,
        }
        .into(),
        Failure::NotRunnable { target } => NotRunnable {
            message: format!("{target} built but is not runnable: it has no executable output"),
            target,
        }
        .into(),
        Failure::Build { target, code } => BuildError {
            message: format!("the build of {target} failed with exit code {code}"),
            target,
            code,
        }
        .into(),
        Failure::ManifestsDisabled { target } => RunError {
            message: format!(
                "{target} has runfiles but Bazel wrote no runfiles manifest for it: \
                 --nobuild_runfile_manifests is in effect, and the runfiles tree is built \
                 from the manifest. Remove it from the rc for this invocation."
            ),
        }
        .into(),
        Failure::Internal(message) => anyhow::anyhow!(message),
    }
}

type SharedOutcome = Shared<BoxFuture<'static, Outcome>>;

/// A built target, materialized and ready to spawn: what `bazel run` would
/// have in hand right before it executes. A script's view of
/// [`bazel::command::run::Runnable`], whose fields it reads through.
#[derive(Debug, Clone, ProvidesStaticType, Display, NoSerialize, Allocative)]
#[display("<bazel.run.Runnable {}>", _0.label)]
pub struct Runnable(#[allocative(skip)] bazel::command::run::Runnable);

impl std::ops::Deref for Runnable {
    type Target = bazel::command::run::Runnable;

    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

starlark_simple_value!(Runnable);

#[starlark_value(type = "bazel.run.Runnable")]
impl<'v> StarlarkValue<'v> for Runnable {
    fn get_methods() -> Option<&'static Methods> {
        static RES: MethodsStatic = MethodsStatic::new("runnable_methods", runnable_methods);
        Some(RES.methods())
    }
}

impl FutureAlloc for Runnable {
    fn alloc_value_fut<'v>(self: Box<Self>, heap: Heap<'v>) -> values::Value<'v> {
        self.alloc_value(heap)
    }
}

/// Where a spawned process is told the workspace and the CLI live, for this
/// task.
fn dirs_from_env(env: &Env) -> Dirs {
    Dirs {
        bazel_root: env.bazel_root_dir.clone(),
        aspect_root: env.aspect_root_dir.clone(),
        git_root: env.git_root_dir.clone(),
        cwd: std::env::current_dir().unwrap_or_else(|_| env.bazel_root_dir.clone()),
    }
}

fn os(path: &Path) -> String {
    path.to_string_lossy().into_owned()
}

#[starlark_module]
fn runnable_methods(registry: &mut MethodsBuilder) {
    /// The target, as it was asked for.
    #[starlark(attribute)]
    fn label<'v>(this: values::Value<'v>) -> anyhow::Result<String> {
        Ok(this.downcast_ref::<Runnable>().unwrap().label.clone())
    }

    /// The main repository's name inside runfiles trees: `_main` under
    /// bzlmod, the `WORKSPACE` name otherwise.
    #[starlark(attribute)]
    fn workspace_name<'v>(this: values::Value<'v>) -> anyhow::Result<String> {
        Ok(this
            .downcast_ref::<Runnable>()
            .unwrap()
            .workspace_name
            .clone())
    }

    /// The executable's absolute path on disk.
    #[starlark(attribute)]
    fn path<'v>(this: values::Value<'v>) -> anyhow::Result<String> {
        Ok(os(&this.downcast_ref::<Runnable>().unwrap().path))
    }

    /// The executable's path relative to the execution root, e.g.
    /// `bazel-out/k8-fastbuild/bin/pkg/tool`.
    #[starlark(attribute)]
    fn exec_path<'v>(this: values::Value<'v>) -> anyhow::Result<String> {
        Ok(this.downcast_ref::<Runnable>().unwrap().exec_path.clone())
    }

    /// The runfiles directory's path relative to the execution root:
    /// `exec_path + ".runfiles"`.
    #[starlark(attribute)]
    fn runfiles_exec_path<'v>(this: values::Value<'v>) -> anyhow::Result<String> {
        Ok(this
            .downcast_ref::<Runnable>()
            .unwrap()
            .runfiles_exec_path
            .clone())
    }

    /// The input runfiles manifest Bazel wrote next to the executable, or
    /// `None` for an executable without runfiles support.
    #[starlark(attribute)]
    fn runfiles_manifest<'v>(this: values::Value<'v>) -> anyhow::Result<NoneOr<String>> {
        Ok(NoneOr::from_option(
            this.downcast_ref::<Runnable>()
                .unwrap()
                .runfiles_manifest
                .as_deref()
                .map(os),
        ))
    }

    /// The materialized runfiles tree, `<path>.runfiles`, or `None` when the
    /// executable has no runfiles. It exists by the time you hold this value.
    #[starlark(attribute)]
    fn runfiles_dir<'v>(this: values::Value<'v>) -> anyhow::Result<NoneOr<String>> {
        Ok(NoneOr::from_option(
            this.downcast_ref::<Runnable>()
                .unwrap()
                .runfiles_dir
                .as_deref()
                .map(os),
        ))
    }

    /// `runfiles_dir/<workspace_name>`, the working directory `bazel run`
    /// starts the process in, or `None` without a runfiles tree.
    #[starlark(attribute)]
    fn workspace_dir<'v>(this: values::Value<'v>) -> anyhow::Result<NoneOr<String>> {
        Ok(NoneOr::from_option(
            this.downcast_ref::<Runnable>()
                .unwrap()
                .workspace_dir
                .as_deref()
                .map(os),
        ))
    }

    /// The target's `args` attribute, location- and make-variable-expanded
    /// and shell-tokenized the way `bazel run` does it.
    #[starlark(attribute)]
    fn args<'v>(this: values::Value<'v>) -> anyhow::Result<Vec<String>> {
        Ok(this.downcast_ref::<Runnable>().unwrap().args.clone())
    }

    /// `RunEnvironmentInfo.environment`: the variables the rule asks to be
    /// set when the target runs.
    #[starlark(attribute)]
    fn env<'v>(this: values::Value<'v>) -> anyhow::Result<Vec<(String, String)>> {
        Ok(this.downcast_ref::<Runnable>().unwrap().env.clone())
    }

    /// `RunEnvironmentInfo.inherited_environment`: variable names the rule
    /// asks to be passed through from the invoking environment. They are, by
    /// virtue of the process inheriting the CLI's environment.
    #[starlark(attribute)]
    fn inherited_env<'v>(this: values::Value<'v>) -> anyhow::Result<Vec<String>> {
        Ok(this
            .downcast_ref::<Runnable>()
            .unwrap()
            .inherited_env
            .clone())
    }

    /// The target's `tags`.
    #[starlark(attribute)]
    fn tags<'v>(this: values::Value<'v>) -> anyhow::Result<Vec<String>> {
        Ok(this.downcast_ref::<Runnable>().unwrap().tags.clone())
    }

    /// Absolute paths of every file in the target's default output group,
    /// the executable among them.
    #[starlark(attribute)]
    fn default_outputs<'v>(this: values::Value<'v>) -> anyhow::Result<Vec<String>> {
        Ok(this
            .downcast_ref::<Runnable>()
            .unwrap()
            .default_outputs
            .iter()
            .map(|p| os(p))
            .collect())
    }

    /// A `std.process.Command` set up the way `bazel run` would start this
    /// target, for callers that want to spawn it themselves.
    ///
    /// The argv is the target's `args` attribute followed by `args`. The
    /// environment is `RunEnvironmentInfo`, then `BUILD_WORKSPACE_DIRECTORY`,
    /// `BUILD_WORKING_DIRECTORY` and the `ASPECT_*` directories, then the
    /// runfiles variables when the target has a runfiles tree, then `env`,
    /// where a `None` value removes the variable. `cwd` defaults to
    /// `workspace_dir`, the directory `bazel run` uses, else the directory
    /// the task was invoked from. The command spawns in its own process
    /// group, so cancelling its token stops everything it started.
    ///
    /// ```python
    /// child = r.command(args = ["--port", "8080"]).stdout("piped").spawn()
    /// ```
    fn command<'v>(
        this: values::Value<'v>,
        #[starlark(require = named, default = UnpackList::default())] args: UnpackList<String>,
        #[starlark(require = named, default = UnpackDictEntries::default())] env: UnpackDictEntries<
            String,
            NoneOr<String>,
        >,
        #[starlark(require = named, default = NoneOr::None)] cwd: NoneOr<String>,
        eval: &mut Evaluator<'v, '_, '_>,
    ) -> anyhow::Result<Command> {
        let runnable = this.downcast_ref_err::<Runnable>().into_anyhow_result()?;
        let dirs = dirs_from_env(Env::from_eval(eval)?);
        let env: Vec<(String, Option<String>)> = env
            .entries
            .into_iter()
            .map(|(k, v)| (k, v.into_option()))
            .collect();
        let cmd = runnable.command(
            &dirs,
            &args.items,
            &env,
            cwd.into_option().as_deref().map(Path::new),
        );
        Ok(Command::from_std(cmd, true))
    }
}

/// What `run` spawns once its target is ready.
struct SpawnSpec {
    args: Vec<String>,
    env: Vec<(String, Option<String>)>,
    cwd: Option<PathBuf>,
    stdin: Stdio,
    stdout: Stdio,
    stderr: Stdio,
    token: CancellationToken,
    signals: Arc<Signals>,
    tx: oneshot::Sender<Result<Child, Failure>>,
}

/// Start the process `run` asked for and hand it to the `child` future.
fn spawn_child(runnable: &bazel::command::run::Runnable, dirs: &Dirs, spec: SpawnSpec) {
    let SpawnSpec {
        args,
        env,
        cwd,
        stdin,
        stdout,
        stderr,
        token,
        signals,
        tx,
    } = spec;
    let mut cmd = runnable.command(dirs, &args, &env, cwd.as_deref());
    cmd.stdin(stdin).stdout(stdout).stderr(stderr);
    let result = children::spawn(&mut cmd, &signals, &token, Some(0), children::Kind::Process)
        .map(|(child, bound)| Child::from_parts(child, bound))
        .map_err(|e| {
            Failure::Internal(format!("failed to spawn {}: {e}", runnable.path.display()))
        });
    let _ = tx.send(result);
}

/// The flags both verbs add to the rc-resolved command line, and the
/// runfiles mode the rc implies; see [`bazel::command::run::prepare_flags`].
fn prepare_flags(
    signals: &Arc<Signals>,
    flags: &mut Vec<String>,
    startup_flags: &[String],
) -> anyhow::Result<SymlinksMode> {
    let runner = super::runner::SignalRunner(signals);
    bazel::command::run::prepare_flags(&bazel::server::Server::new(&runner, startup_flags), flags)
        .map_err(|e| match e {
            PrepareError::Refused(message) => RunError { message }.into(),
            PrepareError::Other(e) => e,
        })
}

/// What both verbs share after the build is spawned.
struct Started {
    build: Build,
    shared: HashMap<String, SharedOutcome>,
    completed: mpsc::Receiver<String>,
    done: Arc<AtomicBool>,
}

#[allow(clippy::too_many_arguments)]
fn start<'v>(
    this: values::Value<'v>,
    env: &Env,
    targets: &[String],
    build_events: (bool, Vec<super::build::BuildEventSink>, Vec<BuildEventIter>),
    execution_log: (bool, Vec<ExecLogSink>, Vec<super::iter::ExecLogIter>),
    rc: Option<bazel::rc::BazelRC>,
    directory: Option<String>,
    (build_stdout, build_stderr): (Stdio, Stdio),
    announce: super::build::AnnounceSpawn,
    cancellation: &CancellationToken,
    spawn: Option<SpawnSpec>,
) -> anyhow::Result<Started> {
    super::require_claimed_flags(this)?;
    // `rc` is the only build input: the one given, else the active one from
    // `use_rc`, else none, in which case Bazel reads its own rc files.
    let (mut flags, startup_flags) = match rc.or_else(|| super::read_active_rc(this)) {
        Some(rc) => {
            let (startup, cmd) = rc.resolve_for_command("build")?;
            (cmd, startup)
        }
        None => (vec![], super::read_startup_flags(this)?),
    };
    let mode = prepare_flags(&env.signals, &mut flags, &startup_flags)?;

    let dirs = dirs_from_env(env);
    let package = dirs.current_package();
    let mut given = HashMap::new();
    let mut keys = Vec::new();
    for target in targets {
        let key = apparent_label(target, &package);
        if given.insert(key.clone(), target.clone()).is_none() {
            keys.push(key);
        }
    }

    let mut senders = HashMap::new();
    let mut shared = HashMap::new();
    for key in &keys {
        let (tx, rx) = oneshot::channel::<Outcome>();
        senders.insert(key.clone(), tx);
        let fut: BoxFuture<'static, Outcome> = rx
            .map(|r| {
                r.unwrap_or_else(|_| {
                    Err(Failure::Internal(
                        "the build ended before this target was resolved".to_owned(),
                    ))
                })
            })
            .boxed();
        shared.insert(key.clone(), fut.shared());
    }

    let (mut sinks, mut iters) = (build_events.1, build_events.2);
    let ours = BuildEventIter::new(env.signals.clone(), None, None);
    iters.push(ours.clone());
    let _ = &mut sinks;

    let build = Build::spawn(
        "build",
        targets.iter().cloned(),
        (true, sinks, iters),
        execution_log,
        false,
        flags,
        startup_flags,
        build_stdout,
        build_stderr,
        directory,
        announce,
        env.signals.clone(),
        cancellation,
        env.rt.clone(),
    )?;

    let recv = ours
        .take_recv()
        .ok_or_else(|| anyhow::anyhow!("the run tracker's BES subscription was not bound"))?;
    let (completed_tx, completed_rx) = mpsc::channel();
    let done = Arc::new(AtomicBool::new(false));
    // The one target `run` starts is spawned the moment it settles, from the
    // tracker's thread, or told why it never will be.
    let on_settled = spawn.map(|spec| -> OnSettled {
        Box::new(move |settled| match settled {
            Ok(runnable) => spawn_child(runnable, &dirs, spec),
            Err(failure) => {
                let _ = spec.tx.send(Err(failure.clone()));
            }
        })
    });
    let tracker = Tracker::new(
        given,
        keys.iter().cloned(),
        senders,
        completed_tx,
        mode,
        on_settled,
    );
    let done_flag = done.clone();
    std::thread::Builder::new()
        .name("bazel-run-tracker".to_owned())
        .spawn(move || tracker.run(recv, done_flag))?;

    Ok(Started {
        build,
        shared,
        completed: completed_rx,
        done,
    })
}

fn runnable_future<'v>(shared: &SharedOutcome) -> FutureOf<'v, Runnable> {
    let fut = shared.clone();
    FutureOf::from_future(async move { fut.await.map(Runnable).map_err(failure_error) })
}

/// One target built for running: the handle `ctx.bazel.run` returns.
#[derive(ProvidesStaticType, Display, Trace, NoSerialize, Allocative)]
#[display("<bazel.run.Run {label}>")]
pub struct Run {
    label: String,
    #[allocative(skip)]
    build: Build,
    #[allocative(skip)]
    runnable: SharedOutcome,
    #[allocative(skip)]
    child: RefCell<Option<oneshot::Receiver<Result<Child, Failure>>>>,
    #[allocative(skip)]
    status: RefCell<Option<BuildStatus>>,
    #[allocative(skip)]
    signals: Arc<Signals>,
}

impl std::fmt::Debug for Run {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Run").field("label", &self.label).finish()
    }
}

impl<'v> AllocValue<'v> for Run {
    fn alloc_value(self, heap: Heap<'v>) -> values::Value<'v> {
        heap.alloc_complex_no_freeze(self)
    }
}

#[starlark_value(type = "bazel.run.Run")]
impl<'v> StarlarkValue<'v> for Run {
    fn get_methods() -> Option<&'static Methods> {
        static RES: MethodsStatic = MethodsStatic::new("run_methods", run_methods);
        Some(RES.methods())
    }
}

impl Run {
    fn build_status(&self) -> anyhow::Result<BuildStatus> {
        if let Some(status) = self.status.borrow().as_ref() {
            return Ok(status.clone());
        }
        let status = self.build.wait_status()?;
        self.status.replace(Some(status.clone()));
        Ok(status)
    }

    fn take_child(&self) -> anyhow::Result<oneshot::Receiver<Result<Child, Failure>>> {
        self.child.borrow_mut().take().ok_or_else(|| {
            anyhow::anyhow!(
                "run.child was already taken for {}: the process is handed out once",
                self.label
            )
        })
    }
}

#[starlark_module]
fn run_methods(registry: &mut MethodsBuilder) {
    /// The target, as it was asked for.
    #[starlark(attribute)]
    fn label<'v>(this: values::Value<'v>) -> anyhow::Result<String> {
        Ok(this.downcast_ref::<Run>().unwrap().label.clone())
    }

    /// The built target, once it is built and its runfiles tree exists.
    /// Fails with `bazel.run.BuildError`, `bazel.run.NotBuilt` or
    /// `bazel.run.NotRunnable`. Each access is a new future over the same
    /// result.
    #[starlark(attribute)]
    fn runnable<'v>(this: values::Value<'v>) -> anyhow::Result<FutureOf<'v, Runnable>> {
        Ok(runnable_future(
            &this.downcast_ref::<Run>().unwrap().runnable,
        ))
    }

    /// The target's process, spawned the moment the target was ready. A
    /// `std.process.Child`: `wait`, `try_wait`, `stdout`, `kill` as usual.
    /// Fails the way `runnable` does when the target did not build. Handed
    /// out once; a second access is an error.
    #[starlark(attribute)]
    fn child<'v>(this: values::Value<'v>) -> anyhow::Result<FutureOf<'v, Child>> {
        let run = this.downcast_ref_err::<Run>().into_anyhow_result()?;
        let rx = run.take_child()?;
        Ok(FutureOf::from_future(async move {
            match rx.await {
                Ok(Ok(child)) => Ok(child),
                Ok(Err(failure)) => Err(failure_error(failure)),
                Err(_) => Err(anyhow::anyhow!(
                    "the build ended before the process was spawned"
                )),
            }
        }))
    }

    /// Block until the process exits and return its `std.process.ExitStatus`,
    /// then join the build. Raises the build errors `runnable` would. Takes
    /// `child`, so use one or the other.
    fn wait<'v>(
        this: values::Value<'v>,
        eval: &mut Evaluator<'v, '_, '_>,
    ) -> anyhow::Result<crate::engine::std::process::ExitStatus> {
        let run = this.downcast_ref_err::<Run>().into_anyhow_result()?;
        let env: &Env = Env::from_eval(eval)?;
        let rx = run.take_child()?;
        let child = match env.signals.block(rx) {
            Ok(Ok(Ok(child))) => child,
            Ok(Ok(Err(failure))) => return Err(failure_error(failure)),
            Ok(Err(_)) => anyhow::bail!("the build ended before the process was spawned"),
            Err(exit) => return Err(exit.into()),
        };
        let status = child
            .wait_status(None)?
            .ok_or_else(|| anyhow::anyhow!("a wait without a timeout reports a status"))?;
        run.build_status()?;
        Ok(crate::engine::std::process::ExitStatus(status))
    }

    /// Block until the build finishes and return its `BuildStatus`. The
    /// process, if it was spawned, keeps running.
    fn status<'v>(this: values::Value<'v>) -> anyhow::Result<BuildStatus> {
        this.downcast_ref_err::<Run>()
            .into_anyhow_result()?
            .build_status()
    }

    /// The invocation id every gRPC BES sink filed this build under, or `""`
    /// without sinks. As on `bazel.build.Build`.
    #[starlark(attribute)]
    fn sink_invocation_id<'v>(this: values::Value<'v>) -> anyhow::Result<String> {
        let run = this.downcast_ref_err::<Run>().into_anyhow_result()?;
        Ok(run.build.sink_invocation_id())
    }

    /// The build's cancellation token: cancelling it interrupts Bazel. The
    /// process has its own, the one passed as `cancellation =`.
    #[starlark(attribute)]
    fn cancellation<'v>(this: values::Value<'v>) -> anyhow::Result<cancellation::Token> {
        let run = this.downcast_ref_err::<Run>().into_anyhow_result()?;
        Ok(cancellation::Token::new(
            run.build.cancellation_token().clone(),
            run.signals.clone(),
        ))
    }
}

/// Many targets built for running: the handle `ctx.bazel.run_all` returns.
#[derive(ProvidesStaticType, Display, Trace, NoSerialize, Allocative)]
#[display("<bazel.run.Runs>")]
pub struct Runs {
    #[allocative(skip)]
    build: Build,
    #[allocative(skip)]
    shared: HashMap<String, SharedOutcome>,
    #[allocative(skip)]
    given: HashMap<String, String>,
    #[allocative(skip)]
    completed: Mutex<mpsc::Receiver<String>>,
    #[allocative(skip)]
    queue: RefCell<VecDeque<String>>,
    #[allocative(skip)]
    done: Arc<AtomicBool>,
    #[allocative(skip)]
    status: RefCell<Option<BuildStatus>>,
    #[allocative(skip)]
    package: String,
    #[allocative(skip)]
    signals: Arc<Signals>,
    #[allocative(skip)]
    popped: Cell<usize>,
}

impl std::fmt::Debug for Runs {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Runs")
            .field("targets", &self.shared.len())
            .finish()
    }
}

impl<'v> AllocValue<'v> for Runs {
    fn alloc_value(self, heap: Heap<'v>) -> values::Value<'v> {
        heap.alloc_complex_no_freeze(self)
    }
}

#[starlark_value(type = "bazel.run.Runs")]
impl<'v> StarlarkValue<'v> for Runs {
    fn get_methods() -> Option<&'static Methods> {
        static RES: MethodsStatic = MethodsStatic::new("runs_methods", runs_methods);
        Some(RES.methods())
    }
}

impl Runs {
    fn drain(&self) {
        let rx = self.completed.lock().unwrap();
        let mut queue = self.queue.borrow_mut();
        while let Ok(key) = rx.try_recv() {
            queue.push_back(key);
        }
    }
}

#[starlark_module]
fn runs_methods(registry: &mut MethodsBuilder) {
    /// The next target to finish building, as `(label, Future[Runnable])`
    /// in completion order, or `None` when none is ready yet. The future is
    /// already settled: `block()` returns at once, with the `Runnable` or
    /// one of `bazel.run.BuildError`, `NotBuilt`, `NotRunnable`.
    fn try_pop<'v>(
        this: values::Value<'v>,
    ) -> anyhow::Result<NoneOr<(String, FutureOf<'v, Runnable>)>> {
        let runs = this.downcast_ref_err::<Runs>().into_anyhow_result()?;
        runs.drain();
        let Some(key) = runs.queue.borrow_mut().pop_front() else {
            return Ok(NoneOr::None);
        };
        runs.popped.set(runs.popped.get() + 1);
        let label = runs.given.get(&key).cloned().unwrap_or_else(|| key.clone());
        let shared = runs
            .shared
            .get(&key)
            .ok_or_else(|| anyhow::anyhow!("no slot for {key}"))?;
        Ok(NoneOr::Other((label, runnable_future(shared))))
    }

    /// Every target has settled and been popped.
    #[starlark(attribute)]
    fn done<'v>(this: values::Value<'v>) -> anyhow::Result<bool> {
        let runs = this.downcast_ref_err::<Runs>().into_anyhow_result()?;
        runs.drain();
        Ok(runs.done.load(Ordering::SeqCst)
            && runs.queue.borrow().is_empty()
            && runs.popped.get() >= runs.shared.len())
    }

    /// The `Future[Runnable]` for one of the targets, by label, however it
    /// was spelled. Blocks on `block()` until that target settles.
    fn get<'v>(
        this: values::Value<'v>,
        #[starlark(require = pos)] label: String,
    ) -> anyhow::Result<FutureOf<'v, Runnable>> {
        let runs = this.downcast_ref_err::<Runs>().into_anyhow_result()?;
        let key = apparent_label(&label, &runs.package);
        let shared = runs.shared.get(&key).ok_or_else(|| {
            anyhow::anyhow!("{label} is not one of the targets this run_all was started with")
        })?;
        Ok(runnable_future(shared))
    }

    /// Block until the build finishes and return its `BuildStatus`.
    fn wait<'v>(this: values::Value<'v>) -> anyhow::Result<BuildStatus> {
        let runs = this.downcast_ref_err::<Runs>().into_anyhow_result()?;
        if let Some(status) = runs.status.borrow().as_ref() {
            return Ok(status.clone());
        }
        let status = runs.build.wait_status()?;
        runs.status.replace(Some(status.clone()));
        Ok(status)
    }

    /// The invocation id every gRPC BES sink filed this build under, or `""`
    /// without sinks. As on `bazel.build.Build`.
    #[starlark(attribute)]
    fn sink_invocation_id<'v>(this: values::Value<'v>) -> anyhow::Result<String> {
        let runs = this.downcast_ref_err::<Runs>().into_anyhow_result()?;
        Ok(runs.build.sink_invocation_id())
    }

    /// The build's cancellation token: cancelling it interrupts Bazel.
    #[starlark(attribute)]
    fn cancellation<'v>(this: values::Value<'v>) -> anyhow::Result<cancellation::Token> {
        let runs = this.downcast_ref_err::<Runs>().into_anyhow_result()?;
        Ok(cancellation::Token::new(
            runs.build.cancellation_token().clone(),
            runs.signals.clone(),
        ))
    }
}

type StdioArg<'v> = Option<Either<values::StringValue<'v>, Writable>>;
use either::Either;

fn stdio_arg(arg: StdioArg<'_>, field: &str) -> anyhow::Result<Stdio> {
    match arg {
        None => Ok(Stdio::inherit()),
        Some(io) => stdio_from(io, field),
    }
}

#[starlark_module]
pub(crate) fn run_natives(registry: &mut MethodsBuilder) {
    /// Build one target and run it, the way `bazel run` does, without
    /// changing any build option.
    ///
    /// The build is a plain `bazel build` under `rc`, the only build input.
    /// It injects command options only: the `runinfo` aspect that captures
    /// the executable, the `args` attribute and `RunEnvironmentInfo`, and
    /// `--remote_download_outputs=toplevel` when `rc` resolves to `minimal`,
    /// the same download set Bazel's run command computes. The runfiles tree
    /// is built from the manifest afterwards, so `--build_runfile_links` is
    /// left as the rc has it and the analysis cache survives.
    /// `--nobuild_runfile_manifests` is refused with `bazel.run.Error`, as
    /// `bazel run` refuses it: before the build when `rc` shows it, and at
    /// resolution otherwise, since the aspect reports whether Bazel wrote
    /// the manifest.
    ///
    /// The process starts the moment the target is built: argv is the
    /// target's `args` followed by `args`, the environment is
    /// `RunEnvironmentInfo` plus the `BUILD_*`, `ASPECT_*` and runfiles
    /// variables plus `env` (a `None` value removes a variable), and the
    /// working directory is `cwd`, else the runfiles workspace directory
    /// `bazel run` uses. The process is its own process group bound to
    /// `cancellation`: cancelling the token stops it and everything it
    /// started.
    ///
    /// Returns a `Run`. `run.wait()` blocks through the build and the
    /// process and returns the process's exit status; `run.child` is the
    /// `std.process.Child` for callers that want to drive it; `run.runnable`
    /// describes what was built. A failed build surfaces on those as
    /// `bazel.run.BuildError`, a target the build never produced as
    /// `bazel.run.NotBuilt`, one with no executable as
    /// `bazel.run.NotRunnable`.
    ///
    /// # Arguments
    ///
    /// * `target` - The label to run.
    /// * `args` - Arguments appended after the target's own `args` attribute.
    /// * `env` - Variables set on the process after everything `bazel run`
    ///   sets; `None` removes one.
    /// * `cwd` - Working directory; the runfiles workspace directory when omitted.
    /// * `stdin`, `stdout`, `stderr` - `"inherit"` (the default), `"piped"`,
    ///   `"null"`, or a writable file stream from `ctx.std.fs.create(...)`.
    /// * `rc` - The `RunCommand` the build runs under; the active one from
    ///   `use_rc` when omitted.
    /// * `directory`, `build_events`, `execution_log`, `announce_version`,
    ///   `announce_command` - As on `ctx.bazel.build`. The build's own
    ///   stdout and stderr are inherited; the stdio knobs here are the
    ///   process's.
    /// * `cancellation` - The token the process and the build are bound to;
    ///   `ctx.cancellation.root` unless given.
    ///
    /// ```python
    /// run = ctx.bazel.run("//svc:api", args = ["--port", "8080"], rc = rc)
    /// err, status = catch(run.wait, types = [bazel.run.BuildError])
    /// if err:
    ///     return err.code
    /// return status.code
    /// ```
    fn run<'v>(
        this: values::Value<'v>,
        #[starlark(require = pos)] target: values::StringValue<'v>,
        #[starlark(require = named, default = UnpackList::default())] args: UnpackList<String>,
        #[starlark(require = named, default = UnpackDictEntries::default())] env: UnpackDictEntries<
            String,
            NoneOr<String>,
        >,
        #[starlark(require = named, default = NoneOr::None)] cwd: NoneOr<String>,
        #[starlark(require = named)] stdin: Option<Either<values::StringValue<'v>, Writable>>,
        #[starlark(require = named)] stdout: Option<Either<values::StringValue<'v>, Writable>>,
        #[starlark(require = named)] stderr: Option<Either<values::StringValue<'v>, Writable>>,
        #[starlark(require = named, default = Either::Left(false))] build_events: Either<
            bool,
            UnpackList<Either<super::build::BuildEventSink, BuildEventIter>>,
        >,
        #[starlark(require = named, default = Either::Left(false))] execution_log: Either<
            bool,
            UnpackList<Either<ExecLogSink, super::iter::ExecLogIter>>,
        >,
        #[starlark(require = named, default = NoneOr::None)] rc: NoneOr<&'v bazel::rc::BazelRC>,
        #[starlark(require = named, default = NoneOr::None)] directory: NoneOr<String>,
        #[starlark(require = named, default = false)] announce_version: bool,
        #[starlark(require = named, default = false)] announce_command: bool,
        #[starlark(require = named, default = NoneOr::None)] cancellation: NoneOr<
            &'v cancellation::Token,
        >,
        eval: &mut Evaluator<'v, '_, '_>,
    ) -> anyhow::Result<Run> {
        let env_: &Env = Env::from_eval(eval)?;
        let token = super::cancellation_token(&env_.signals, cancellation);
        let (child_tx, child_rx) = oneshot::channel();
        let spec = SpawnSpec {
            args: args.items,
            env: env
                .entries
                .into_iter()
                .map(|(k, v)| (k, v.into_option()))
                .collect(),
            cwd: cwd.into_option().map(PathBuf::from),
            stdin: stdio_arg(stdin, "stdin")?,
            stdout: stdio_arg(stdout, "stdout")?,
            stderr: stdio_arg(stderr, "stderr")?,
            token: token.clone(),
            signals: env_.signals.clone(),
            tx: child_tx,
        };
        let label = target.as_str().to_owned();
        let started = start(
            this,
            env_,
            std::slice::from_ref(&label),
            super::partition_build_events(build_events),
            super::partition_execution_log(execution_log),
            rc.into_option().cloned(),
            directory.into_option(),
            (Stdio::inherit(), Stdio::inherit()),
            super::build::AnnounceSpawn {
                version: announce_version,
                command: announce_command,
            },
            &token,
            Some(spec),
        )?;
        let key = apparent_label(&label, &dirs_from_env(env_).current_package());
        let runnable = started
            .shared
            .get(&key)
            .cloned()
            .ok_or_else(|| anyhow::anyhow!("no slot for {label}"))?;
        Ok(Run {
            label,
            build: started.build,
            runnable,
            child: RefCell::new(Some(child_rx)),
            status: RefCell::new(None),
            signals: env_.signals.clone(),
        })
    }

    /// Build many targets for running, in one build, and describe each as it
    /// finishes. Spawns nothing: each `Runnable` carries what `bazel run`
    /// would set up, and `runnable.command(...)` is the process to spawn.
    ///
    /// The build is the one `ctx.bazel.run` makes, over all of `targets`,
    /// under `rc`. Pass `--keep_going` in the rc when one target's failure
    /// should not stop the others. `runs.try_pop()` yields
    /// `(label, Future[Runnable])` in the order targets finish, so a target
    /// can be started while the rest of the build continues; `runs.get(label)`
    /// is the same future by name; `runs.wait()` is the build's status.
    ///
    /// # Arguments
    ///
    /// * `targets` - The labels to build.
    /// * `rc` - The `RunCommand` the build runs under; the active one from
    ///   `use_rc` when omitted.
    /// * `stdout`, `stderr`, `stdio` - The build's streams, as on
    ///   `ctx.bazel.build`; nothing is spawned here, so these are Bazel's.
    /// * `directory`, `build_events`, `execution_log`, `announce_version`,
    ///   `announce_command` - As on `ctx.bazel.build`.
    /// * `cancellation` - The token the build is bound to;
    ///   `ctx.cancellation.root` unless given.
    ///
    /// ```python
    /// runs = ctx.bazel.run_all(labels, rc = rc)
    /// for _ in sleep_iter(50):
    ///     popped = runs.try_pop()
    ///     if popped:
    ///         label, fut = popped
    ///         err, r = fut.catch(bazel.run.BuildError).block()
    ///         children[label] = err or r.command().spawn()
    ///     if runs.done:
    ///         break
    /// status = runs.wait()
    /// ```
    fn run_all<'v>(
        this: values::Value<'v>,
        #[starlark(require = pos)] targets: UnpackList<String>,
        #[starlark(require = named, default = Either::Left(false))] build_events: Either<
            bool,
            UnpackList<Either<super::build::BuildEventSink, BuildEventIter>>,
        >,
        #[starlark(require = named, default = Either::Left(false))] execution_log: Either<
            bool,
            UnpackList<Either<ExecLogSink, super::iter::ExecLogIter>>,
        >,
        #[starlark(require = named, default = NoneOr::None)] rc: NoneOr<&'v bazel::rc::BazelRC>,
        #[starlark(require = named)] stdout: Option<NoneOr<Writable>>,
        #[starlark(require = named)] stderr: Option<NoneOr<Writable>>,
        #[starlark(require = named, default = NoneOr::None)] stdio: NoneOr<
            crate::engine::std::io::Stdio,
        >,
        #[starlark(require = named, default = NoneOr::None)] directory: NoneOr<String>,
        #[starlark(require = named, default = false)] announce_version: bool,
        #[starlark(require = named, default = false)] announce_command: bool,
        #[starlark(require = named, default = NoneOr::None)] cancellation: NoneOr<
            &'v cancellation::Token,
        >,
        eval: &mut Evaluator<'v, '_, '_>,
    ) -> anyhow::Result<Runs> {
        let env: &Env = Env::from_eval(eval)?;
        let token = super::cancellation_token(&env.signals, cancellation);
        let dirs = dirs_from_env(env);
        let package = dirs.current_package();
        let mut given = HashMap::new();
        for target in &targets.items {
            given.insert(apparent_label(target, &package), target.clone());
        }
        let started = start(
            this,
            env,
            &targets.items,
            super::partition_build_events(build_events),
            super::partition_execution_log(execution_log),
            rc.into_option().cloned(),
            directory.into_option(),
            super::resolve_stdio(stdio, stdout, stderr)?,
            super::build::AnnounceSpawn {
                version: announce_version,
                command: announce_command,
            },
            &token,
            None,
        )?;
        Ok(Runs {
            build: started.build,
            shared: started.shared,
            given,
            completed: Mutex::new(started.completed),
            queue: RefCell::new(VecDeque::new()),
            done: started.done,
            status: RefCell::new(None),
            package,
            signals: env.signals.clone(),
            popped: Cell::new(0),
        })
    }
}

#[starlark_module]
pub(crate) fn register_run_types(globals: &mut starlark::environment::GlobalsBuilder) {
    const Runnable: starlark::values::starlark_value_as_type::StarlarkValueAsType<Runnable> =
        starlark::values::starlark_value_as_type::StarlarkValueAsType::new();
    const Run: starlark::values::starlark_value_as_type::StarlarkValueAsType<Run> =
        starlark::values::starlark_value_as_type::StarlarkValueAsType::new();
    const Runs: starlark::values::starlark_value_as_type::StarlarkValueAsType<Runs> =
        starlark::values::starlark_value_as_type::StarlarkValueAsType::new();
}

pub(crate) fn register_run_errors(globals: &mut starlark::environment::GlobalsBuilder) {
    use crate::engine::error::NativeError;
    globals.set("Error", RunError::native_type().error_type());
    globals.set("BuildError", BuildError::native_type().error_type());
    globals.set("NotBuilt", NotBuilt::native_type().error_type());
    globals.set("NotRunnable", NotRunnable::native_type().error_type());
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn what_crosses_threads_is_send() {
        fn is_send<T: Send>() {}
        is_send::<Runnable>();
        is_send::<Child>();
        is_send::<SpawnSpec>();
    }
}
