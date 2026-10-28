//! `bazel run`, done by building the target and then running what it built:
//! what the `runinfo` aspect records about a target, and which of its outputs
//! is the executable.

//!
//! Both verbs start one `bazel build` and inject only command options: the
//! `runinfo` aspect (what `bazel run` reads but no provider exposes: the
//! executable, the expanded `args` attribute, `RunEnvironmentInfo`), its
//! output group, and `--remote_download_outputs=toplevel` when the rc resolves
//! to `minimal`, which is the download set Bazel's own RUN mode computes. The
//! runfiles tree is built from the manifest after the fact ([`crate::runfiles`]),
//! so `--build_runfile_links` is never touched and the analysis cache survives.
//!
//! A [`Tracker`] follows the build's BES stream and settles one slot per
//! target the moment its default and `aspect_runinfo` output groups, and every
//! fileset they reference, have arrived.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, mpsc};

use serde::Deserialize;
use tokio::sync::oneshot;

use super::flags::{download_outputs_sufficient, effective_bool, effective_download_outputs};
use crate::aspect::{self, Aspect};
use crate::bes::BuildEventEnvelope;
use crate::bes::outputs::{Outputs, Ready, Unsettled};
use crate::fanout::Subscriber;
use crate::runfiles::{self, SymlinksMode};
use crate::server::Server;

/// The `.bzl` source of the aspect that captures what `bazel run` reads.
pub const RUNINFO_BZL: &str = include_str!("runinfo.bzl");
const RUNINFO_SYMBOL: &str = "runinfo";
/// The output group the `runinfo` aspect writes its capture to.
pub const RUNINFO_OUTPUT_GROUP: &str = "aspect_runinfo";
const RUNINFO_JSON_SUFFIX: &str = ".aspect_runinfo.json";
/// The main repository's name inside runfiles when the build did not say.
pub const DEFAULT_WORKSPACE_NAME: &str = "_main";

pub fn runinfo_aspect() -> Aspect {
    Aspect {
        implementation: RUNINFO_BZL.to_owned(),
        symbol: RUNINFO_SYMBOL.to_owned(),
        parameters: vec![],
        output_groups: vec![RUNINFO_OUTPUT_GROUP.to_owned()],
    }
}

/// How a target's slot settled when it did not settle with a [`Runnable`].
#[derive(Debug, Clone)]
pub enum Failure {
    NotBuilt { target: String },
    NotRunnable { target: String },
    Build { target: String, code: i32 },
    ManifestsDisabled { target: String },
    Internal(String),
}

pub type Outcome = Result<Runnable, Failure>;

/// A built target, materialized and ready to spawn: what `bazel run` would
/// have in hand right before it executes.
#[derive(Debug, Clone)]
pub struct Runnable {
    pub label: String,
    pub workspace_name: String,
    pub exec_path: String,
    pub runfiles_exec_path: String,
    pub path: PathBuf,
    pub runfiles_manifest: Option<PathBuf>,
    pub runfiles_dir: Option<PathBuf>,
    pub workspace_dir: Option<PathBuf>,
    pub args: Vec<String>,
    pub env: Vec<(String, String)>,
    pub inherited_env: Vec<String>,
    pub tags: Vec<String>,
    pub default_outputs: Vec<PathBuf>,
}

/// Where a spawned process is told the workspace and the CLI live.
#[derive(Debug, Clone)]
pub struct Dirs {
    pub bazel_root: PathBuf,
    pub aspect_root: PathBuf,
    pub git_root: Option<PathBuf>,
    pub cwd: PathBuf,
}

impl Dirs {
    /// The workspace-relative package the task was invoked from, `""` at the
    /// root or outside the workspace. Relative labels resolve against it.
    pub fn current_package(&self) -> String {
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
    pub fn command(
        &self,
        dirs: &Dirs,
        args: &[String],
        env: &[(String, Option<String>)],
        cwd: Option<&Path>,
    ) -> Command {
        let mut cmd = Command::new(&self.path);
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

/// Why `run` refused before the build started, or could not get ready to.
#[derive(Debug)]
pub enum PrepareError {
    /// The rc asks for something running a target cannot honour.
    Refused(String),
    Other(anyhow::Error),
}

/// The flag that decides whether Bazel uploads the files BEP names to the
/// remote cache before it writes the event that names them.
const UPLOAD_STRATEGY: &str = "experimental_build_event_upload_strategy";

/// The flags both verbs add to the rc-resolved command line, without the
/// aspect, and the runfiles mode the rc implies. Every injected flag is a
/// command option; none is part of the configuration, which is what keeps the
/// analysis cache.
///
/// * `--remote_download_outputs=toplevel` when the rc resolves to `minimal`:
///   the executable and its runfiles have to be on disk.
/// * `--experimental_build_event_upload_strategy=local`, always. The tracker
///   reads local paths out of BEP and nothing else from the upload. With a
///   remote cache Bazel instead uploads every file BEP names and holds the
///   event that names it until the upload is done, reporting it as
///   `bytestream://`. The build is then long over, and the client gone —
///   builds run `fully_async` — before the target's completion and the build's
///   exit code are written, so the target never settles and is reported as a
///   failed build (aspect-build/aspect-cli#1604). A caller's own strategy
///   cannot be honoured: `run` does not work under `remote`. Added last, so it
///   wins over an earlier literal flag and over rc content.
fn inject_flags(flags: &mut Vec<String>) -> Result<SymlinksMode, PrepareError> {
    if effective_bool(flags, "build_runfile_manifests") == Some(false) {
        return Err(PrepareError::Refused(
            "--nobuild_runfile_manifests is incompatible with running a target: \
             the runfiles tree is built from the manifest. Remove it from the rc \
             for this invocation."
                .to_owned(),
        ));
    }
    let mode = SymlinksMode::from_enable_runfiles(effective_bool(flags, "enable_runfiles"));
    if !download_outputs_sufficient(effective_download_outputs(flags).as_deref()) {
        flags.push("--remote_download_outputs=toplevel".to_owned());
    }
    flags.push(format!("--{UPLOAD_STRATEGY}=local"));
    Ok(mode)
}

/// [`inject_flags`], then the `runinfo` aspect's flags.
pub fn prepare_flags(
    server: &Server,
    flags: &mut Vec<String>,
) -> Result<SymlinksMode, PrepareError> {
    let mode = inject_flags(flags)?;
    flags.extend(aspect::materialize(server, &[runinfo_aspect()]).map_err(PrepareError::Other)?);
    Ok(mode)
}

/// Turn a ready target's files into a `Runnable`: pick the executable, read
/// the aspect's capture, and build the runfiles tree.
pub fn resolve(
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

/// Why a target the build ended on did not produce a `Runnable`.
///
/// A target whose own completion reported failure, or that was aborted, is a
/// build failure whatever else happened. One that completed with its default
/// outputs but never got an `aspect_runinfo` group built fine and has no
/// executable: the aspect returns nothing for a library or a filegroup. One
/// the build never reported at all was not built, and when the build itself
/// failed that is the likeliest reason, so the failure carries Bazel's code.
pub fn classify_unsettled(target: String, failed: bool, has_default: bool, code: i32) -> Failure {
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

/// What the host does with the one target it wants started as soon as it is
/// ready: handed the `Runnable`, or the failure that settled the slot instead.
pub type OnSettled = Box<dyn FnOnce(Result<&Runnable, &Failure>) + Send>;

/// What the tracker thread does with each target the BES stream settles.
pub struct Tracker {
    /// Apparent key → label as the caller wrote it.
    given: HashMap<String, String>,
    outputs: Outputs,
    senders: HashMap<String, oneshot::Sender<Outcome>>,
    completed: mpsc::Sender<String>,
    mode: SymlinksMode,
    on_settled: Option<OnSettled>,
}

impl Tracker {
    /// Follow `keys` (apparent labels), settling each one's slot in `senders`
    /// and announcing it on `completed`. `on_settled`, when given, is called
    /// for the first target to settle.
    pub fn new(
        given: HashMap<String, String>,
        keys: impl IntoIterator<Item = String>,
        senders: HashMap<String, oneshot::Sender<Outcome>>,
        completed: mpsc::Sender<String>,
        mode: SymlinksMode,
        on_settled: Option<OnSettled>,
    ) -> Self {
        Self {
            given,
            outputs: Outputs::new(
                keys.into_iter().collect(),
                RUNINFO_OUTPUT_GROUP,
                DEFAULT_WORKSPACE_NAME.to_owned(),
            ),
            senders,
            completed,
            mode,
            on_settled,
        }
    }

    /// Read `recv` to its end, then fail whatever never settled. Sets `done`
    /// last.
    pub fn run(mut self, recv: Subscriber<Arc<BuildEventEnvelope>>, done: Arc<AtomicBool>) {
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
        let on_settled = self.on_settled.take();
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
            if let Some(on_settled) = on_settled {
                on_settled(outcome.as_ref());
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
            if let Some(on_settled) = self.on_settled.take() {
                on_settled(Err(&failure));
            }
            if let Some(tx) = self.senders.remove(&key) {
                let _ = tx.send(Err(failure));
            }
            let _ = self.completed.send(key);
        }
    }
}

/// Extensions an executable output may carry when the rule names its
/// entrypoint after the target plus a suffix, most to least common.
const SCRIPT_EXTENSIONS: &[&str] = &[".sh", ".bash", ".exe", ".bat"];

/// What the `runinfo` aspect writes per runnable target.
#[derive(Debug, Deserialize, Default)]
pub struct RunInfo {
    #[serde(default)]
    pub executable: String,
    #[serde(default)]
    pub args: Vec<String>,
    #[serde(default)]
    pub environment: serde_json::Map<String, serde_json::Value>,
    #[serde(default)]
    pub inherited_environment: Vec<String>,
    #[serde(default)]
    pub tags: Vec<String>,
    /// The target has runfiles support (a non-empty default runfiles set).
    #[serde(default = "yes")]
    pub runfiles: bool,
    /// Bazel wrote `<exe>.runfiles_manifest` for it.
    #[serde(default = "yes")]
    pub runfiles_manifest: bool,
}

fn yes() -> bool {
    true
}

/// The executable among `candidates`: the aspect's `files_to_run.executable`
/// basename when the aspect ran, else a file named after the target, else
/// the target name plus a script extension, else the first file with the
/// executable bit.
pub fn pick_executable(
    candidates: &[PathBuf],
    runinfo: Option<&RunInfo>,
    target_name: &str,
) -> Option<PathBuf> {
    let basename = |p: &PathBuf| p.file_name().map(|n| n.to_string_lossy().into_owned());
    if let Some(info) = runinfo {
        let exe = info.executable.rsplit('/').next().unwrap_or("");
        if !exe.is_empty() {
            if let Some(found) = candidates
                .iter()
                .find(|c| basename(c).as_deref() == Some(exe))
            {
                return Some(found.clone());
            }
        }
    }
    if let Some(found) = candidates
        .iter()
        .find(|c| basename(c).as_deref() == Some(target_name))
    {
        return Some(found.clone());
    }
    for ext in SCRIPT_EXTENSIONS {
        let name = format!("{target_name}{ext}");
        if let Some(found) = candidates
            .iter()
            .find(|c| basename(c).as_deref() == Some(&name))
        {
            return Some(found.clone());
        }
    }
    candidates.iter().find(|c| is_executable(c)).cloned()
}

#[cfg(unix)]
fn is_executable(path: &Path) -> bool {
    use std::os::unix::fs::PermissionsExt;
    std::fs::metadata(path)
        .map(|m| m.is_file() && m.permissions().mode() & 0o111 != 0)
        .unwrap_or(false)
}

#[cfg(not(unix))]
fn is_executable(path: &Path) -> bool {
    path.is_file()
}

#[cfg(test)]
mod run_tests {
    use super::*;

    fn flags(items: &[&str]) -> Vec<String> {
        items.iter().map(|s| s.to_string()).collect()
    }

    /// aspect-build/aspect-cli#1604: with a remote cache BEP is held until its
    /// files upload, so a build that runs a target must not wait on that.
    #[test]
    fn a_run_asks_bazel_not_to_upload_what_it_reads_locally() {
        let mut f = flags(&["--remote_cache=grpcs://cache.example.com"]);
        inject_flags(&mut f).unwrap();
        assert!(
            f.contains(&"--experimental_build_event_upload_strategy=local".to_owned()),
            "{f:?}"
        );
    }

    /// `run` does not work under `remote`, so a caller's choice is overridden:
    /// the injected flag comes last, and the last literal flag wins.
    #[test]
    fn a_strategy_the_caller_chose_is_overridden_by_the_last_flag() {
        let name = "--experimental_build_event_upload_strategy=";
        for chosen in ["remote", "local"] {
            let mut f = flags(&[&format!("{name}{chosen}")]);
            inject_flags(&mut f).unwrap();
            assert_eq!(
                f.last().map(String::as_str),
                Some("--experimental_build_event_upload_strategy=local")
            );
        }
        // Chosen in an rc file, which renders as an override: a literal
        // flag outranks it.
        let mut f = flags(&[
            "--default_override=0:common=--experimental_build_event_upload_strategy=remote",
        ]);
        inject_flags(&mut f).unwrap();
        assert_eq!(
            crate::command::flags::effective_value(&f, "experimental_build_event_upload_strategy")
                .as_deref(),
            Some("local")
        );
    }

    #[test]
    fn minimal_downloads_are_raised_and_a_manifestless_rc_is_refused() {
        let mut f = flags(&["--remote_download_minimal"]);
        inject_flags(&mut f).unwrap();
        assert!(f.contains(&"--remote_download_outputs=toplevel".to_owned()));
        let mut refused = flags(&["--nobuild_runfile_manifests"]);
        assert!(matches!(
            inject_flags(&mut refused),
            Err(PrepareError::Refused(_))
        ));
    }

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

    #[test]
    fn a_stream_that_ends_fails_the_target_and_hands_the_callback_the_failure() {
        let (tx, mut rx) = oneshot::channel::<Outcome>();
        let (completed_tx, completed_rx) = mpsc::channel();
        let (seen_tx, seen_rx) = mpsc::channel::<String>();
        let tracker = Tracker::new(
            HashMap::from([("//:t".to_owned(), "//:t".to_owned())]),
            ["//:t".to_owned()],
            HashMap::from([("//:t".to_owned(), tx)]),
            completed_tx,
            SymlinksMode::from_enable_runfiles(None),
            Some(Box::new(move |settled| {
                let _ = seen_tx.send(format!("{:?}", settled.err()));
            })),
        );
        // A subscriber whose producer is already gone: the stream ends at once.
        let broadcaster = crate::fanout::Broadcaster::<Arc<BuildEventEnvelope>>::new();
        let sub = broadcaster.subscribe();
        broadcaster.close();
        let done = Arc::new(AtomicBool::new(false));
        tracker.run(sub, done.clone());

        assert!(done.load(Ordering::SeqCst));
        assert_eq!(completed_rx.recv().unwrap(), "//:t");
        // Bazel never said how the build ended, so it is assumed to have
        // failed with exit code 1, and the target is a build failure.
        assert!(matches!(
            rx.try_recv(),
            Ok(Err(Failure::Build { code: 1, .. }))
        ));
        let seen = seen_rx.recv().unwrap();
        assert!(seen.contains("Build"), "{seen}");
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn executable_choice_prefers_the_aspect_then_the_name_then_extensions() {
        let tmp = tempfile::tempdir().unwrap();
        let mk = |name: &str, exec: bool| {
            let p = tmp.path().join(name);
            std::fs::write(&p, "").unwrap();
            #[cfg(unix)]
            {
                use std::os::unix::fs::PermissionsExt;
                std::fs::set_permissions(
                    &p,
                    std::fs::Permissions::from_mode(if exec { 0o755 } else { 0o644 }),
                )
                .unwrap();
            }
            let _ = exec;
            p
        };
        let other = mk("other", true);
        let tool_sh = mk("tool.sh", false);
        let tool = mk("tool", false);
        let candidates = vec![other.clone(), tool_sh.clone(), tool.clone()];
        let info = RunInfo {
            executable: "pkg/tool.sh".to_owned(),
            ..Default::default()
        };
        assert_eq!(
            pick_executable(&candidates, Some(&info), "tool"),
            Some(tool_sh.clone())
        );
        assert_eq!(pick_executable(&candidates, None, "tool"), Some(tool));
        assert_eq!(
            pick_executable(&[other.clone(), tool_sh.clone()], None, "tool"),
            Some(tool_sh)
        );
        #[cfg(unix)]
        assert_eq!(pick_executable(&[other.clone()], None, "tool"), Some(other));
    }

    #[test]
    fn runinfo_json_decodes_what_the_aspect_writes() {
        let text = r#"{"executable":"pkg/tool","args":["a","b c"],"environment":{"K":"V"},"inherited_environment":["HOME"],"tags":["manual"]}"#;
        let info: RunInfo = serde_json::from_str(text).unwrap();
        assert_eq!(info.executable, "pkg/tool");
        assert_eq!(info.args, vec!["a", "b c"]);
        assert_eq!(info.environment.get("K").unwrap(), "V");
        assert_eq!(info.inherited_environment, vec!["HOME"]);
        assert_eq!(info.tags, vec!["manual"]);
        // Fields an older aspect did not write default to the common case.
        assert!(info.runfiles && info.runfiles_manifest);
        let text = r#"{"executable":"pkg/tool","runfiles":true,"runfiles_manifest":false}"#;
        let info: RunInfo = serde_json::from_str(text).unwrap();
        assert!(info.runfiles && !info.runfiles_manifest);
    }
}
