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
use std::process::{self, Stdio};
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

use super::aspect::{self, Aspect};
use super::build::{Build, BuildEventIter, BuildStatus};
use crate::engine::r#async::future::{FutureAlloc, FutureOf};
use crate::engine::cancellation::{self, Signals};
use crate::engine::children;
use crate::engine::error::native_error;
use crate::engine::std::process::{Child, Command, stdio_from};
use crate::engine::std::stream::Writable;
use crate::engine::store::Env;
use bazel::bes::BuildEventEnvelope;
use bazel::bes::outputs::{Outputs, Ready, Unsettled};
use bazel::command::flags::{
    download_outputs_sufficient, effective_bool, effective_download_outputs,
};
use bazel::command::run::{RunInfo, pick_executable};
use bazel::fanout::Subscriber;
use bazel::label::apparent_label;
use bazel::runfiles::{self, SymlinksMode};

const RUNINFO_BZL: &str = include_str!("runinfo.bzl");
const RUNINFO_SYMBOL: &str = "runinfo";
const RUNINFO_OUTPUT_GROUP: &str = "aspect_runinfo";
const RUNINFO_JSON_SUFFIX: &str = ".aspect_runinfo.json";
const DEFAULT_WORKSPACE_NAME: &str = "_main";

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

/// How a target's slot settled when it did not settle with a `Runnable`.
#[derive(Debug, Clone)]
enum Failure {
    NotBuilt { target: String },
    NotRunnable { target: String },
    Build { target: String, code: i32 },
    ManifestsDisabled { target: String },
    Internal(String),
}

impl Failure {
    fn into_anyhow(self) -> anyhow::Error {
        match self {
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
}

type Outcome = Result<Runnable, Failure>;
type SharedOutcome = Shared<BoxFuture<'static, Outcome>>;

/// A built target, materialized and ready to spawn: what `bazel run` would
/// have in hand right before it executes.
#[derive(Debug, Clone, ProvidesStaticType, Display, NoSerialize, Allocative)]
#[display("<bazel.run.Runnable {label}>")]
pub struct Runnable {
    label: String,
    workspace_name: String,
    exec_path: String,
    runfiles_exec_path: String,
    path: PathBuf,
    runfiles_manifest: Option<PathBuf>,
    runfiles_dir: Option<PathBuf>,
    workspace_dir: Option<PathBuf>,
    args: Vec<String>,
    env: Vec<(String, String)>,
    inherited_env: Vec<String>,
    tags: Vec<String>,
    default_outputs: Vec<PathBuf>,
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

/// Where a spawned process is told the workspace and the CLI live.
#[derive(Debug, Clone)]
struct Dirs {
    bazel_root: PathBuf,
    aspect_root: PathBuf,
    git_root: Option<PathBuf>,
    cwd: PathBuf,
}

impl Dirs {
    fn from_env(env: &Env) -> Self {
        Self {
            bazel_root: env.bazel_root_dir.clone(),
            aspect_root: env.aspect_root_dir.clone(),
            git_root: env.git_root_dir.clone(),
            cwd: std::env::current_dir().unwrap_or_else(|_| env.bazel_root_dir.clone()),
        }
    }

    /// The workspace-relative package the task was invoked from, `""` at the
    /// root or outside the workspace. Relative labels resolve against it.
    fn current_package(&self) -> String {
        match self.cwd.strip_prefix(&self.bazel_root) {
            Ok(rel) => rel.to_string_lossy().replace('\\', "/"),
            Err(_) => String::new(),
        }
    }
}

fn os(path: &Path) -> String {
    path.to_string_lossy().into_owned()
}

impl Runnable {
    /// The process `bazel run` would start: the target's `args` attribute
    /// ahead of `args`, `RunEnvironmentInfo` first so the variables below win,
    /// the `BUILD_*` and `ASPECT_*` variables, the runfiles variables when a
    /// tree exists (with `RUNFILES_MANIFEST_FILE` removed, so a launcher never
    /// inherits the CLI's own manifest), then `env`, and `cwd` defaulting to
    /// the runfiles workspace directory.
    fn command(
        &self,
        dirs: &Dirs,
        args: &[String],
        env: &[(String, Option<String>)],
        cwd: Option<&Path>,
    ) -> process::Command {
        let mut cmd = process::Command::new(&self.path);
        cmd.args(&self.args).args(args);
        for (k, v) in &self.env {
            cmd.env(k, v);
        }
        cmd.env("BUILD_WORKSPACE_DIRECTORY", &dirs.bazel_root)
            .env("BUILD_WORKING_DIRECTORY", &dirs.cwd)
            .env("ASPECT_WORKING_DIRECTORY", &dirs.cwd)
            .env("ASPECT_ROOT_DIRECTORY", &dirs.aspect_root)
            .env("ASPECT_BAZEL_ROOT_DIRECTORY", &dirs.bazel_root)
            .env(
                "ASPECT_GIT_ROOT_DIRECTORY",
                dirs.git_root.as_deref().unwrap_or(Path::new("")),
            );
        if let Some(dir) = &self.runfiles_dir {
            cmd.env("RUNFILES_DIR", dir).env("JAVA_RUNFILES", dir);
            if let Some(ws) = &self.workspace_dir {
                cmd.env("RUNFILES_WORKSPACE_DIR", ws);
            }
            cmd.env_remove("RUNFILES_MANIFEST_FILE");
        }
        for (k, v) in env {
            match v {
                Some(v) => cmd.env(k, v),
                None => cmd.env_remove(k),
            };
        }
        let dir = cwd
            .map(Path::to_path_buf)
            .or_else(|| self.workspace_dir.clone())
            .unwrap_or_else(|| dirs.cwd.clone());
        cmd.current_dir(dir);
        cmd
    }
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
        let dirs = Dirs::from_env(Env::from_eval(eval)?);
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

/// What the tracker thread does with each target the BES stream settles.
struct Tracker {
    /// Apparent key → label as the caller wrote it.
    given: HashMap<String, String>,
    outputs: Outputs,
    senders: HashMap<String, oneshot::Sender<Outcome>>,
    completed: mpsc::Sender<String>,
    mode: SymlinksMode,
    dirs: Dirs,
    spawn: Option<SpawnSpec>,
}

impl Tracker {
    fn run(mut self, recv: Subscriber<Arc<BuildEventEnvelope>>, done: Arc<AtomicBool>) {
        while let Ok(envelope) = recv.recv() {
            for ready in self.outputs.handle(&envelope.event) {
                self.settle_resolved(ready);
            }
        }
        self.finish();
        done.store(true, Ordering::SeqCst);
    }

    fn settle_resolved(&mut self, ready: Ready) {
        let Ready {
            key,
            default_files,
            extra_files: runinfo_files,
        } = ready;
        let Some(tx) = self.senders.remove(&key) else {
            return;
        };
        let label = self.given.get(&key).cloned().unwrap_or_else(|| key.clone());
        let completed = self.completed.clone();
        let spawn = self.spawn.take();
        let dirs = self.dirs.clone();
        let exec_root = self.outputs.exec_root().map(Path::to_path_buf);
        let workspace_name = self.outputs.workspace_name().to_owned();
        let mode = self.mode;
        // Materializing a runfiles tree can take a while; a thread per target
        // keeps one target's tree from delaying the next target's readiness.
        std::thread::spawn(move || {
            let outcome = resolve(
                &label,
                exec_root.as_deref(),
                &workspace_name,
                mode,
                default_files,
                runinfo_files,
            );
            if let Some(spec) = spawn {
                match &outcome {
                    Ok(runnable) => spawn_child(runnable, &dirs, spec),
                    Err(failure) => {
                        let _ = spec.tx.send(Err(failure.clone()));
                    }
                }
            }
            let _ = tx.send(outcome);
            let _ = completed.send(key);
        });
    }

    /// The stream closed: everything still pending did not settle on its own.
    fn finish(mut self) {
        let (code, unsettled) = self.outputs.finish();
        for Unsettled {
            key,
            failed,
            has_default,
        } in unsettled
        {
            let target = self.given.get(&key).cloned().unwrap_or_else(|| key.clone());
            let failure = classify_unsettled(target, failed, has_default, code);
            if let Some(spec) = self.spawn.take() {
                let _ = spec.tx.send(Err(failure.clone()));
            }
            if let Some(tx) = self.senders.remove(&key) {
                let _ = tx.send(Err(failure));
            }
            let _ = self.completed.send(key);
        }
    }
}

/// Why a target the build ended on did not produce a `Runnable`.
///
/// A target whose own completion reported failure, or that was aborted, is a
/// build failure whatever else happened. One that completed with its default
/// outputs but never got an `aspect_runinfo` group built fine and has no
/// executable: the aspect returns nothing for a library or a filegroup. One
/// the build never reported at all was not built, and when the build itself
/// failed that is the likeliest reason, so the failure carries Bazel's code.
fn classify_unsettled(target: String, failed: bool, has_default: bool, code: i32) -> Failure {
    if failed {
        Failure::Build { target, code }
    } else if has_default {
        Failure::NotRunnable { target }
    } else if code != 0 {
        Failure::Build { target, code }
    } else {
        Failure::NotBuilt { target }
    }
}

/// Start the process `run` asked for and hand it to the `child` future.
fn spawn_child(runnable: &Runnable, dirs: &Dirs, spec: SpawnSpec) {
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

/// Turn a ready target's files into a `Runnable`: pick the executable, read
/// the aspect's capture, and build the runfiles tree.
fn resolve(
    label: &str,
    exec_root: Option<&Path>,
    workspace_name: &str,
    mode: SymlinksMode,
    default_files: Vec<PathBuf>,
    runinfo_files: Vec<PathBuf>,
) -> Outcome {
    let runinfo = runinfo_files
        .iter()
        .find(|p| p.to_string_lossy().ends_with(RUNINFO_JSON_SUFFIX))
        .and_then(|p| std::fs::read_to_string(p).ok())
        .and_then(|text| serde_json::from_str::<RunInfo>(&text).ok());
    let target_name = label.rsplit(':').next().unwrap_or(label).to_owned();
    let Some(path) = pick_executable(&default_files, runinfo.as_ref(), &target_name) else {
        return Err(Failure::NotRunnable {
            target: label.to_owned(),
        });
    };
    let runinfo = runinfo.unwrap_or_default();
    let workspace_name = if workspace_name.is_empty() {
        DEFAULT_WORKSPACE_NAME
    } else {
        workspace_name
    };

    // The aspect says whether Bazel wrote a manifest in this build. A
    // manifest or `<exe>.runfiles` directory found on disk without that is
    // from an earlier build and is not trusted: with runfiles but no manifest
    // nothing can run, and without runfiles there is no tree to build.
    if runinfo.runfiles && !runinfo.runfiles_manifest {
        return Err(Failure::ManifestsDisabled {
            target: label.to_owned(),
        });
    }
    let (runfiles_manifest, runfiles_dir) = if runinfo.runfiles_manifest {
        let manifest = runfiles::input_manifest(&path);
        let dir = match runfiles::materialize(&path, workspace_name, mode) {
            Ok(dir) => dir,
            Err(err) => {
                return Err(Failure::Internal(format!(
                    "failed to build the runfiles tree for {label}: {err}"
                )));
            }
        };
        (manifest, dir)
    } else {
        (None, None)
    };
    let workspace_dir = runfiles_dir.as_ref().map(|d| d.join(workspace_name));

    let exec_path = exec_root
        .and_then(|root| path.strip_prefix(root).ok())
        .map(|p| p.to_string_lossy().replace('\\', "/"))
        .unwrap_or_else(|| os(&path));
    let runfiles_exec_path = format!("{exec_path}{}", runfiles::RUNFILES_DIR_EXT);

    Ok(Runnable {
        label: label.to_owned(),
        workspace_name: workspace_name.to_owned(),
        exec_path,
        runfiles_exec_path,
        path,
        runfiles_manifest,
        runfiles_dir,
        workspace_dir,
        args: runinfo.args,
        env: runinfo
            .environment
            .into_iter()
            .map(|(k, v)| {
                let v = match v {
                    serde_json::Value::String(s) => s,
                    other => other.to_string(),
                };
                (k, v)
            })
            .collect(),
        inherited_env: runinfo.inherited_environment,
        tags: runinfo.tags,
        default_outputs: default_files,
    })
}

fn runinfo_aspect() -> Aspect {
    Aspect {
        implementation: RUNINFO_BZL.to_owned(),
        symbol: RUNINFO_SYMBOL.to_owned(),
        parameters: vec![],
        output_groups: vec![RUNINFO_OUTPUT_GROUP.to_owned()],
    }
}

/// The flags both verbs add to the rc-resolved command line, and the
/// runfiles mode the rc implies. Every injected flag is a command option;
/// none is part of the configuration, which is what keeps the analysis cache.
fn prepare_flags(
    signals: &Arc<Signals>,
    flags: &mut Vec<String>,
    startup_flags: &[String],
) -> anyhow::Result<SymlinksMode> {
    if effective_bool(flags, "build_runfile_manifests") == Some(false) {
        return Err(RunError {
            message: "--nobuild_runfile_manifests is incompatible with running a target: \
                      the runfiles tree is built from the manifest. Remove it from the rc \
                      for this invocation."
                .to_owned(),
        }
        .into());
    }
    let mode = SymlinksMode::from_enable_runfiles(effective_bool(flags, "enable_runfiles"));
    if !download_outputs_sufficient(effective_download_outputs(flags).as_deref()) {
        flags.push("--remote_download_outputs=toplevel".to_owned());
    }
    flags.extend(aspect::materialize(
        signals,
        &[runinfo_aspect()],
        startup_flags,
    )?);
    Ok(mode)
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
    execution_log: (
        bool,
        Vec<super::sink::execlog::ExecLogSink>,
        Vec<super::iter::ExecLogIter>,
    ),
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

    let dirs = Dirs::from_env(env);
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
                    Failure::Internal("the build ended before this target was resolved".to_owned())
                        .into_outcome()
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
    let tracker = Tracker {
        given,
        outputs: Outputs::new(
            keys.iter().cloned().collect(),
            RUNINFO_OUTPUT_GROUP,
            DEFAULT_WORKSPACE_NAME.to_owned(),
        ),
        senders,
        completed: completed_tx,
        mode,
        dirs,
        spawn,
    };
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

impl Failure {
    fn into_outcome(self) -> Outcome {
        Err(self)
    }
}

fn runnable_future<'v>(shared: &SharedOutcome) -> FutureOf<'v, Runnable> {
    let fut = shared.clone();
    FutureOf::from_future(async move { fut.await.map_err(Failure::into_anyhow) })
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
                Ok(Err(failure)) => Err(failure.into_anyhow()),
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
            Ok(Ok(Err(failure))) => return Err(failure.into_anyhow()),
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
            UnpackList<Either<super::sink::execlog::ExecLogSink, super::iter::ExecLogIter>>,
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
        let key = apparent_label(&label, &Dirs::from_env(env_).current_package());
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
            UnpackList<Either<super::sink::execlog::ExecLogSink, super::iter::ExecLogIter>>,
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
        let dirs = Dirs::from_env(env);
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
    fn unsettled_targets_are_classified_by_their_own_state_first() {
        let t = || "//:t".to_owned();
        assert!(matches!(
            classify_unsettled(t(), true, true, 0),
            Failure::Build { code: 0, .. }
        ));
        assert!(matches!(
            classify_unsettled(t(), false, true, 1),
            Failure::NotRunnable { .. }
        ));
        assert!(matches!(
            classify_unsettled(t(), false, false, 1),
            Failure::Build { code: 1, .. }
        ));
        assert!(matches!(
            classify_unsettled(t(), false, false, 0),
            Failure::NotBuilt { .. }
        ));
    }

    #[test]
    fn what_crosses_threads_is_send() {
        fn is_send<T: Send>() {}
        is_send::<Runnable>();
        is_send::<Child>();
        is_send::<SpawnSpec>();
    }

    #[test]
    fn command_composes_the_bazel_run_environment() {
        let r = Runnable {
            label: "//pkg:tool".into(),
            workspace_name: "_main".into(),
            exec_path: "bazel-out/bin/pkg/tool".into(),
            runfiles_exec_path: "bazel-out/bin/pkg/tool.runfiles".into(),
            path: PathBuf::from("/x/bazel-out/bin/pkg/tool"),
            runfiles_manifest: None,
            runfiles_dir: Some(PathBuf::from("/x/bazel-out/bin/pkg/tool.runfiles")),
            workspace_dir: Some(PathBuf::from("/x/bazel-out/bin/pkg/tool.runfiles/_main")),
            args: vec!["--from-attr".into()],
            env: vec![("FROM_RULE".into(), "1".into())],
            inherited_env: vec![],
            tags: vec![],
            default_outputs: vec![],
        };
        let dirs = Dirs {
            bazel_root: PathBuf::from("/ws"),
            aspect_root: PathBuf::from("/ws"),
            git_root: None,
            cwd: PathBuf::from("/ws/pkg"),
        };
        let cmd = r.command(
            &dirs,
            &["--extra".to_owned()],
            &[
                ("FROM_RULE".to_owned(), None),
                ("MINE".to_owned(), Some("2".to_owned())),
            ],
            None,
        );
        let args: Vec<_> = cmd
            .get_args()
            .map(|a| a.to_string_lossy().into_owned())
            .collect();
        assert_eq!(args, vec!["--from-attr", "--extra"]);
        let envs: HashMap<String, Option<String>> = cmd
            .get_envs()
            .map(|(k, v)| {
                (
                    k.to_string_lossy().into_owned(),
                    v.map(|v| v.to_string_lossy().into_owned()),
                )
            })
            .collect();
        assert_eq!(envs.get("FROM_RULE"), Some(&None));
        assert_eq!(envs.get("MINE"), Some(&Some("2".to_owned())));
        assert_eq!(
            envs.get("RUNFILES_DIR"),
            Some(&Some("/x/bazel-out/bin/pkg/tool.runfiles".to_owned()))
        );
        assert_eq!(envs.get("RUNFILES_MANIFEST_FILE"), Some(&None));
        assert_eq!(
            envs.get("BUILD_WORKSPACE_DIRECTORY"),
            Some(&Some("/ws".to_owned()))
        );
        assert_eq!(
            cmd.get_current_dir(),
            Some(Path::new("/x/bazel-out/bin/pkg/tool.runfiles/_main"))
        );
    }

    #[test]
    fn current_package_is_relative_to_the_workspace() {
        let dirs = Dirs {
            bazel_root: PathBuf::from("/ws"),
            aspect_root: PathBuf::from("/ws"),
            git_root: None,
            cwd: PathBuf::from("/ws/a/b"),
        };
        assert_eq!(dirs.current_package(), "a/b");
        let root = Dirs {
            cwd: PathBuf::from("/ws"),
            ..dirs.clone()
        };
        assert_eq!(root.current_package(), "");
        let outside = Dirs {
            cwd: PathBuf::from("/elsewhere"),
            ..dirs
        };
        assert_eq!(outside.current_package(), "");
    }
}
