use std::cell::RefCell;
use std::collections::HashMap;
use std::process::Stdio;

use allocative::Allocative;
use derive_more::Display;
use either::Either;
use starlark::collections::SmallMap;
use starlark::environment::Methods;
use starlark::environment::MethodsBuilder;
use starlark::environment::MethodsStatic;
use starlark::eval::Evaluator;
use starlark::starlark_simple_value;
use starlark::values;
use starlark::values::NoSerialize;
use starlark::values::ProvidesStaticType;
use starlark::values::Trace;
use starlark::values::Tracer;
use starlark::values::ValueLike;
use starlark::values::dict::UnpackDictEntries;
use starlark::values::list::UnpackList;
use starlark::values::none::NoneOr;
use starlark::values::none::NoneType;
use starlark::values::starlark_value;
use starlark::values::tuple::UnpackTuple;
use starlark::{
    environment::GlobalsBuilder, starlark_module,
    values::starlark_value_as_type::StarlarkValueAsType,
};

use crate::engine::std::io::Stdio as StdStdio;
use crate::engine::store::Env;
use axl_proto;
use axl_types::stream::Writable;

mod aspect;
mod build;
mod cancel;
mod health_check;
mod info;
mod iter;
pub mod live;
mod process;
mod query;
mod sandbox_recovery;
mod sink;
mod stream;

/// Resolve which `bazel` binary to spawn. Honors the `BAZEL_REAL` env var
/// (the bazelisk convention) so wrapped invocations and tests can substitute
/// their own binary; falls back to plain `"bazel"` and lets the OS resolve
/// it via `PATH`.
pub(crate) fn bazel_binary() -> String {
    std::env::var("BAZEL_REAL").unwrap_or_else(|_| "bazel".to_string())
}

/// Build a `Command` for spawning bazel, with the anti-inception env var set.
///
/// When `tools/bazel` wrapper scripts forward to `aspect` for build/test/run,
/// aspect then needs to spawn its OWN child bazel. If `PATH` still has
/// `tools/bazel` first, that child re-enters the wrapper and bounces right
/// back into aspect — inception, repeated forever.
///
/// `ASPECT_CLI_RUNNING=1` breaks the cycle: cooperating wrapper scripts
/// check for it on entry and short-circuit straight to the real bazel
/// (matching the spirit of bazelisk's `BAZELISK_SKIP_WRAPPER`). All bazel
/// spawns from inside aspect go through this helper so the env var is
/// always set, regardless of which call site fired.
pub(crate) fn bazel_command() -> std::process::Command {
    let mut cmd = std::process::Command::new(bazel_binary());
    cmd.env("ASPECT_CLI_RUNNING", "1");
    cmd
}

/// Resolve the `(stdout, stderr)` `Stdio` slots from the Starlark args.
///
/// Tri-state for each per-fd arg: not passed → inherit, Starlark `None` →
/// `/dev/null`, `Writable` → fd-dup that handle into the child. The `stdio`
/// shorthand sets both slots from a single `Stdio` bundle; combining it with
/// per-fd `stdout`/`stderr` is an error so the resolution stays unambiguous.
fn resolve_stdio(
    stdio: NoneOr<StdStdio>,
    stdout: Option<NoneOr<Writable>>,
    stderr: Option<NoneOr<Writable>>,
) -> anyhow::Result<(Stdio, Stdio)> {
    if let NoneOr::Other(s) = stdio {
        if stdout.is_some() || stderr.is_some() {
            anyhow::bail!("stdio cannot be combined with stdout/stderr — pass one or the other");
        }
        return Ok((
            build::writable_to_stdio(&s.stdout)?,
            build::writable_to_stdio(&s.stderr)?,
        ));
    }
    let to_stdio = |spec: Option<NoneOr<Writable>>| -> anyhow::Result<Stdio> {
        match spec {
            None => Ok(Stdio::inherit()),
            Some(NoneOr::None) => Ok(Stdio::null()),
            Some(NoneOr::Other(w)) => Ok(build::writable_to_stdio(&w)?),
        }
    };
    Ok((to_stdio(stdout)?, to_stdio(stderr)?))
}

fn partition_build_events(
    arg: Either<bool, UnpackList<Either<build::BuildEventSink, build::BuildEventIter>>>,
) -> (bool, Vec<build::BuildEventSink>, Vec<build::BuildEventIter>) {
    match arg {
        Either::Left(b) => (b, vec![], vec![]),
        Either::Right(items) => {
            let mut sinks = vec![];
            let mut iters = vec![];
            for item in items.items {
                match item {
                    Either::Left(s) => sinks.push(s),
                    Either::Right(i) => iters.push(i),
                }
            }
            (true, sinks, iters)
        }
    }
}

/// Resolve a mixed list of plain flags and conditional `(flag, constraint)` tuples into
/// a `Vec<String>`. Plain flags are always included; conditional flags are
/// included only when [`constraint_matches`] holds for `version`.
/// Resolve the Bazel version a `RunCommand` should use for version-gated
/// options: an explicit `requested` string when given, else the running Bazel
/// (probed only when `rc` actually carries a gated option, so the common case
/// pays nothing). A failed probe degrades to `None` (assumed-latest).
fn resolve_rc_version(
    requested: Option<String>,
    rc: &bazelrc::BazelRC,
) -> anyhow::Result<Option<semver::Version>> {
    if let Some(s) = requested {
        return Ok(semver::Version::parse(&s).ok());
    }
    if rc.has_version_gated_options() {
        return Ok(info::server_info().ok().and_then(|t| t.1));
    }
    Ok(None)
}

fn resolve_flags<'v>(
    items: &[Either<values::StringValue<'v>, (values::StringValue<'v>, values::StringValue<'v>)>],
    version: Option<&semver::Version>,
) -> anyhow::Result<Vec<String>> {
    let mut result = Vec::with_capacity(items.len());
    for item in items {
        match item {
            Either::Left(s) => result.push(s.as_str().to_string()),
            Either::Right((flag, constraint)) => {
                if constraint_matches(constraint.as_str(), version)? {
                    result.push(flag.as_str().to_string());
                }
            }
        }
    }
    Ok(result)
}

/// Resolve a mixed flag list against the running Bazel version.
///
/// Like [`resolve_flags`], but sources the version itself: a `bazel info`
/// probe is run only when at least one conditional `(flag, constraint)` tuple
/// is present, so unconditional flag lists never pay for the probe. A
/// non-release Bazel reports no version, in which case the constraints resolve
/// against an assumed-latest version (see [`constraint_matches`]).
///
/// Shared by every Bazel subcommand that accepts version-gated flags
/// (`build` / `test` / `query`) so they filter conditional flags identically.
fn resolve_flags_for_running_bazel<'v>(
    items: &[Either<values::StringValue<'v>, (values::StringValue<'v>, values::StringValue<'v>)>],
) -> anyhow::Result<Vec<String>> {
    let version = if items.iter().any(|f| f.is_right()) {
        info::server_info()
            .map_err(|e| anyhow::anyhow!("failed to get Bazel server info: {}", e))?
            .1
    } else {
        None
    };
    resolve_flags(items, version.as_ref())
}

/// Whether a semver `constraint` is satisfied by the running Bazel `version`.
///
/// `version` is `None` when Bazel reports a non-release build (a
/// `development version` has no version number to parse). Such a build is
/// compiled from HEAD, so it is effectively newer than any release: it is
/// matched against a max version, so lower-bound gates (`>=N`, the usual
/// shape) match and upper-bound gates (`<N`) don't.
///
/// An unparseable constraint is a hard error naming the offending value.
///
/// The version's pre-release suffix is ignored: semver `VersionReq` matching
/// treats a pre-release specially (e.g. `8.0.0-rc2` does NOT satisfy `>=8.0.0`),
/// but for flag gating a release candidate should match the same constraints its
/// release will. So `8.0.0-rc2` is compared as `8.0.0`.
fn constraint_matches(constraint: &str, version: Option<&semver::Version>) -> anyhow::Result<bool> {
    let req = semver::VersionReq::parse(constraint)
        .map_err(|e| anyhow::anyhow!("invalid version constraint '{}': {}", constraint, e))?;
    let assumed_latest = semver::Version::new(u64::MAX, u64::MAX, u64::MAX);
    let probe = match version {
        Some(v) if !v.pre.is_empty() => semver::Version::new(v.major, v.minor, v.patch),
        Some(v) => v.clone(),
        None => assumed_latest,
    };
    Ok(req.matches(&probe))
}

#[derive(Debug, Display, ProvidesStaticType, NoSerialize, Allocative)]
#[display("<bazel.Bazel>")]
pub struct Bazel<'v> {
    /// The active `RunCommand` set by `use_rc` (interior-mutable so `use_rc`
    /// can swap it across phases). A per-call `rc=` overrides it. The single
    /// source of both command and startup flags for every Bazel invocation.
    #[allocative(skip)]
    pub active_rc: RefCell<Option<values::Value<'v>>>,
}

unsafe impl<'v> Trace<'v> for Bazel<'v> {
    fn trace(&mut self, tracer: &Tracer<'v>) {
        if let Some(v) = self.active_rc.get_mut() {
            v.trace(tracer);
        }
    }
}

impl<'v> values::AllocValue<'v> for Bazel<'v> {
    fn alloc_value(self, heap: values::Heap<'v>) -> values::Value<'v> {
        heap.alloc_complex(self)
    }
}

impl<'v> values::Freeze for Bazel<'v> {
    type Frozen = FrozenBazel;
    fn freeze(self, freezer: &values::Freezer) -> values::FreezeResult<FrozenBazel> {
        Ok(FrozenBazel {
            active_rc: match self.active_rc.into_inner() {
                Some(v) => Some(v.freeze(freezer)?),
                None => None,
            },
        })
    }
}

#[starlark_value(type = "bazel.Bazel")]
impl<'v> values::StarlarkValue<'v> for Bazel<'v> {
    fn get_methods() -> Option<&'static Methods> {
        static RES: MethodsStatic = MethodsStatic::new();
        RES.methods(bazel_methods)
    }
}

#[derive(Debug, Display, ProvidesStaticType, NoSerialize, Allocative)]
#[display("<bazel.Bazel>")]
pub struct FrozenBazel {
    #[allocative(skip)]
    pub active_rc: Option<values::FrozenValue>,
}

starlark_simple_value!(FrozenBazel);

#[starlark_value(type = "bazel.Bazel")]
impl<'v> values::StarlarkValue<'v> for FrozenBazel {
    type Canonical = Bazel<'v>;
    fn get_methods() -> Option<&'static Methods> {
        static RES: MethodsStatic = MethodsStatic::new();
        RES.methods(bazel_methods)
    }
}

/// The startup flags for an invocation: sourced from the active `RunCommand`
/// (set by `use_rc`), or empty when none is active.
fn read_startup_flags<'v>(this: values::Value<'v>) -> anyhow::Result<Vec<String>> {
    Ok(read_active_rc(this)
        .map(|rc| rc.invocation_startup_flags())
        .unwrap_or_default())
}

/// The active `RunCommand` set via `use_rc`, if any.
fn read_active_rc<'v>(this: values::Value<'v>) -> Option<bazelrc::BazelRC> {
    let slot = if let Some(b) = this.downcast_ref::<Bazel>() {
        *b.active_rc.borrow()
    } else if let Some(b) = this.downcast_ref::<FrozenBazel>() {
        b.active_rc.map(|v| v.to_value())
    } else {
        None
    };
    slot.and_then(|v| v.downcast_ref::<bazelrc::BazelRC>().cloned())
}

/// Resolve the `RunCommand` in effect for an invocation: a per-call `rc=`
/// overrides the active (`use_rc`) one; `None` means no rc (vanilla passthrough,
/// Bazel reads its own `.bazelrc`).
fn effective_rc<'v>(
    this: values::Value<'v>,
    rc_param: NoneOr<values::Value<'v>>,
) -> Option<bazelrc::BazelRC> {
    if let NoneOr::Other(v) = rc_param {
        return v.downcast_ref::<bazelrc::BazelRC>().cloned();
    }
    read_active_rc(this)
}

/// Assemble `(command_flags, startup_flags)` for `command`: when a `RunCommand`
/// is in effect, expand it and append the per-call `flags=` as extras (auto
/// `--ignore_all_rc_files`); otherwise fall back to raw `flags=` + the legacy
/// `ctx.bazel.startup_flags` (Bazel reads its own rc).
fn resolve_invocation_flags<'v>(
    this: values::Value<'v>,
    command: &str,
    rc_param: NoneOr<values::Value<'v>>,
    flags: &[Either<values::StringValue<'v>, (values::StringValue<'v>, values::StringValue<'v>)>],
) -> anyhow::Result<(Vec<String>, Vec<String>)> {
    let extras = resolve_flags_for_running_bazel(flags)?;
    match effective_rc(this, rc_param) {
        Some(rc) => {
            let (startup, mut cmd) = rc.resolve_for_command(command)?;
            cmd.extend(extras);
            Ok((cmd, startup))
        }
        None => Ok((extras, read_startup_flags(this)?)),
    }
}

#[starlark_module]
pub(crate) fn bazel_methods(registry: &mut MethodsBuilder) {
    /// The active `RunCommand` set via `use_rc`, or `None` if none is active.
    /// Lets AXL inspect what build/test/query will run with (flags, startup).
    fn active_rc<'v>(this: values::Value<'v>) -> anyhow::Result<NoneOr<values::Value<'v>>> {
        let slot = if let Some(b) = this.downcast_ref::<Bazel>() {
            *b.active_rc.borrow()
        } else if let Some(b) = this.downcast_ref::<FrozenBazel>() {
            b.active_rc.map(|v| v.to_value())
        } else {
            None
        };
        Ok(match slot {
            Some(v) => NoneOr::Other(v),
            None => NoneOr::None,
        })
    }

    /// Set the active `RunCommand` for subsequent `build` / `test` / `query` on
    /// this context. Callable repeatedly to swap the active run command between
    /// phases. A per-call `rc=` on an invocation overrides the active one; with
    /// no active and no per-call rc, Bazel runs vanilla (reads its own `.bazelrc`).
    fn use_rc<'v>(
        this: values::Value<'v>,
        #[starlark(require = pos)] rc: values::Value<'v>,
    ) -> anyhow::Result<NoneType> {
        if rc.downcast_ref::<bazelrc::BazelRC>().is_none() {
            return Err(anyhow::anyhow!(
                "use_rc expects a RunCommand (from parse_rc / new_rc), got {}",
                rc.get_type()
            ));
        }
        let bazel = this
            .downcast_ref::<Bazel>()
            .ok_or_else(|| anyhow::anyhow!("use_rc: ctx.bazel is frozen"))?;
        bazel.active_rc.replace(Some(rc));
        Ok(NoneType)
    }

    /// Define a Bazel aspect to apply via `ctx.bazel.build(aspects = [...])`
    /// (or `test`).
    ///
    /// The runtime materializes the aspect into `@bazel_tools` — already in the
    /// repo graph — so applying it needs no `--inject_repository` and causes no
    /// analysis-cache churn: the build reuses its warm analysis and layers the
    /// aspect on top. The materialized files live under the Bazel install base
    /// (outside the workspace), so they're invisible to the IDE and VCS.
    ///
    /// # Arguments
    ///
    /// * `implementation` - The `.bzl` source defining the aspect.
    /// * `symbol` - The aspect symbol within `implementation` (the `%name`).
    /// * `parameters` - `{key: value}` passed as `--aspects_parameters`. A value
    ///   that changes per run (e.g. a nonce) mints a fresh `AspectKey`, forcing
    ///   the aspect to re-evaluate without discarding the analysis graph.
    /// * `output_groups` - Output groups to request so the aspect's outputs
    ///   build; each is passed as `--output_groups=+<group>`.
    ///
    /// **Example**
    ///
    /// ```python
    /// hashsum = ctx.bazel.aspect(
    ///     implementation = HASHSUM_BZL,
    ///     symbol = "hashsum",
    ///     parameters = {"nonce": nonce},
    ///     output_groups = ["delivery_hash"],
    /// )
    /// ctx.bazel.build("//my/pkg:target", aspects = [hashsum])
    /// ```
    fn aspect<'v>(
        #[allow(unused_variables)] this: values::Value<'v>,
        #[starlark(require = named)] implementation: String,
        #[starlark(require = named)] symbol: String,
        #[starlark(require = named, default = UnpackDictEntries::default())]
        parameters: UnpackDictEntries<String, String>,
        #[starlark(require = named, default = UnpackList::default())] output_groups: UnpackList<
            String,
        >,
    ) -> anyhow::Result<aspect::Aspect> {
        Ok(aspect::Aspect {
            implementation,
            symbol,
            parameters: parameters.entries,
            output_groups: output_groups.items,
        })
    }

    /// Build one or more Bazel targets.
    ///
    /// Returns a `Build` object. The call does not block — use `.wait()` to
    /// wait for the invocation to finish and retrieve its exit status.
    ///
    /// # Arguments
    ///
    /// * `targets` - One or more Bazel target patterns to build.
    /// * `flags` - Bazel flags. Each element is either a plain `str` that is
    ///   always included, or a `(flag, constraint)` tuple that is included only
    ///   when the running Bazel version satisfies the
    ///   [semver](https://semver.org) constraint. Pre-release versions are
    ///   normalised before matching, so `8.0.0-rc1` is matched by `>=8`. An
    ///   invalid constraint is a hard error.
    /// * `build_events` - Enable the Build Event Protocol stream. Pass `True`
    ///   or a list of `BuildEventSink` values to forward events to remote sinks.
    /// * `workspace_events` - Enable the workspace events stream.
    /// * `execution_logs` - Enable the execution logs stream.
    /// * `stdout` - Per-fd config for the child's stdout. Not passed →
    ///   inherit the parent's stdout. `None` → discard (`/dev/null`). A
    ///   `Writable` (e.g. `ctx.std.io.stderr`, `ctx.std.fs.create("out.log")`)
    ///   → redirect the child's stdout into that handle (the fd is duplicated).
    /// * `stderr` - Per-fd config for the child's stderr. Same shape as `stdout`.
    /// * `stdio` - Shorthand: set both `stdout` and `stderr` from a single
    ///   `Stdio` bundle (typically `ctx.std.io`). Cannot be combined with
    ///   `stdout`/`stderr`.
    /// * `directory` - Working directory for the Bazel invocation; selects
    ///   the workspace / server (used for git-worktree execution).
    /// * `announce_version` - Print an `INFO: Bazel <version>` line before
    ///   spawning. Resolved from the `--announce-bazel-version` task flag.
    /// * `announce_command` - Print an `INFO: Spawning: <command>` line (the
    ///   exact bazel command line) before spawning. Resolved from the
    ///   `--announce-bazel-command` task flag.
    ///
    /// # Arguments
    /// * `execution_log`: Enable Bazel execution log collection. Pass `True` to
    ///   enable the in-memory decoded iterator (accessible via `build.execution_logs()`),
    ///   or pass a list of sinks such as `[execution_log.compact_file(path = "out.binpb.zst")]`
    ///   to write the log to one or more files. Sinks and the iterator can be combined:
    ///   passing a list of sinks still allows calling `build.execution_logs()` to iterate
    ///   entries in-process.
    ///
    /// **Examples**
    ///
    /// ```python
    /// build = ctx.bazel.build("//my/pkg:target", flags = ["--config=release"])
    /// status = build.wait()
    /// ```
    ///
    /// ```python
    /// build = ctx.bazel.build(
    ///     "//my/pkg:target",
    ///     flags = [
    ///         "--config=release",
    ///         ("--notmp_sandbox", ">=8"),
    ///         ("--some_legacy_flag", "<7"),
    ///     ],
    ///     stdout = None,                              # discard child stdout
    ///     stderr = ctx.std.fs.create("bazel.err"),    # redirect stderr to a file
    /// )
    /// status = build.wait()
    /// ```
    fn build<'v>(
        this: values::Value<'v>,
        #[starlark(args)] targets: UnpackTuple<values::StringValue>,
        #[starlark(require = named, default = Either::Left(false))] build_events: Either<
            bool,
            UnpackList<Either<build::BuildEventSink, build::BuildEventIter>>,
        >,
        #[starlark(require = named, default = false)] workspace_events: bool,
        #[starlark(require = named, default = Either::Left(false))] execution_log: Either<
            bool,
            UnpackList<sink::execlog::ExecLogSink>,
        >,
        #[starlark(require = named, default = UnpackList::default())] flags: UnpackList<
            Either<values::StringValue<'v>, (values::StringValue<'v>, values::StringValue<'v>)>,
        >,
        #[starlark(require = named, default = NoneOr::None)] rc: NoneOr<values::Value<'v>>,
        #[starlark(require = named)] stdout: Option<NoneOr<Writable>>,
        #[starlark(require = named)] stderr: Option<NoneOr<Writable>>,
        #[starlark(require = named, default = NoneOr::None)] stdio: NoneOr<StdStdio>,
        #[starlark(require = named, default = NoneOr::None)] directory: NoneOr<String>,
        #[starlark(require = named, default = false)] announce_version: bool,
        #[starlark(require = named, default = false)] announce_command: bool,
        #[starlark(require = named, default = UnpackList::default())] aspects: UnpackList<
            aspect::Aspect,
        >,
        eval: &mut Evaluator<'v, '_, '_>,
    ) -> anyhow::Result<build::Build> {
        let build_events = partition_build_events(build_events);
        let execution_log = match execution_log {
            Either::Left(b) => (b, vec![]),
            Either::Right(sinks) => (true, sinks.items),
        };
        let (mut resolved_flags, resolved_startup_flags) =
            resolve_invocation_flags(this, "build", rc, &flags.items)?;
        resolved_flags.extend(aspect::materialize(
            &aspects.items,
            &resolved_startup_flags,
        )?);
        let env = Env::from_eval(eval)?;
        let (stdout, stderr) = resolve_stdio(stdio, stdout, stderr)?;
        let build = build::Build::spawn(
            "build",
            targets.items.iter().map(|f| f.as_str().to_string()),
            build_events,
            execution_log,
            workspace_events,
            resolved_flags,
            resolved_startup_flags,
            stdout,
            stderr,
            directory.into_option(),
            build::AnnounceSpawn {
                version: announce_version,
                command: announce_command,
            },
            env.rt.clone(),
        )?;
        Ok(build)
    }

    /// Build and test one or more Bazel targets.
    ///
    /// Returns a `Build` object. The call does not block — use `.wait()` to
    /// wait for the invocation to finish and retrieve its exit status.
    ///
    /// # Arguments
    ///
    /// * `targets` - One or more Bazel target patterns to test.
    /// * `flags` - Bazel flags. Each element is either a plain `str` that is
    ///   always included, or a `(flag, constraint)` tuple that is included only
    ///   when the running Bazel version satisfies the
    ///   [semver](https://semver.org) constraint. Pre-release versions are
    ///   normalised before matching, so `8.0.0-rc1` is matched by `>=8`. An
    ///   invalid constraint is a hard error.
    /// * `build_events` - Enable the Build Event Protocol stream. Pass `True`
    ///   or a list of `BuildEventSink` values to forward events to remote sinks.
    /// * `workspace_events` - Enable the workspace events stream.
    /// * `execution_logs` - Enable the execution logs stream.
    /// * `stdout` - Per-fd config for the child's stdout. Not passed →
    ///   inherit the parent's stdout. `None` → discard (`/dev/null`). A
    ///   `Writable` (e.g. `ctx.std.io.stderr`, `ctx.std.fs.create("out.log")`)
    ///   → redirect the child's stdout into that handle (the fd is duplicated).
    /// * `stderr` - Per-fd config for the child's stderr. Same shape as `stdout`.
    /// * `stdio` - Shorthand: set both `stdout` and `stderr` from a single
    ///   `Stdio` bundle (typically `ctx.std.io`). Cannot be combined with
    ///   `stdout`/`stderr`.
    /// * `directory` - Working directory for the Bazel invocation; selects
    ///   the workspace / server (used for git-worktree execution).
    /// * `announce_version` - Print an `INFO: Bazel <version>` line before
    ///   spawning. Resolved from the `--announce-bazel-version` task flag.
    /// * `announce_command` - Print an `INFO: Spawning: <command>` line (the
    ///   exact bazel command line) before spawning. Resolved from the
    ///   `--announce-bazel-command` task flag.
    ///
    /// # Arguments
    /// * `execution_log`: Enable Bazel execution log collection. Pass `True` to
    ///   enable the in-memory decoded iterator (accessible via `build.execution_logs()`),
    ///   or pass a list of sinks such as `[execution_log.compact_file(path = "out.binpb.zst")]`
    ///   to write the log to one or more files. Sinks and the iterator can be combined:
    ///   passing a list of sinks still allows calling `build.execution_logs()` to iterate
    ///   entries in-process.
    ///
    /// **Examples**
    ///
    /// ```python
    /// test = ctx.bazel.test("//my/pkg:test", flags = ["--test_output=errors"])
    /// status = test.wait()
    /// ```
    ///
    /// ```python
    /// test = ctx.bazel.test(
    ///     "//my/pkg:test",
    ///     flags = [
    ///         "--test_output=errors",
    ///         ("--notmp_sandbox", ">=8"),
    ///         ("--some_legacy_flag", "<7"),
    ///     ],
    /// )
    /// status = test.wait()
    /// ```
    fn test<'v>(
        this: values::Value<'v>,
        #[starlark(args)] targets: UnpackTuple<values::StringValue>,
        #[starlark(require = named, default = Either::Left(false))] build_events: Either<
            bool,
            UnpackList<Either<build::BuildEventSink, build::BuildEventIter>>,
        >,
        #[starlark(require = named, default = false)] workspace_events: bool,
        #[starlark(require = named, default = Either::Left(false))] execution_log: Either<
            bool,
            UnpackList<sink::execlog::ExecLogSink>,
        >,
        #[starlark(require = named, default = UnpackList::default())] flags: UnpackList<
            Either<values::StringValue<'v>, (values::StringValue<'v>, values::StringValue<'v>)>,
        >,
        #[starlark(require = named, default = NoneOr::None)] rc: NoneOr<values::Value<'v>>,
        #[starlark(require = named)] stdout: Option<NoneOr<Writable>>,
        #[starlark(require = named)] stderr: Option<NoneOr<Writable>>,
        #[starlark(require = named, default = NoneOr::None)] stdio: NoneOr<StdStdio>,
        #[starlark(require = named, default = NoneOr::None)] directory: NoneOr<String>,
        #[starlark(require = named, default = false)] announce_version: bool,
        #[starlark(require = named, default = false)] announce_command: bool,
        #[starlark(require = named, default = UnpackList::default())] aspects: UnpackList<
            aspect::Aspect,
        >,
        eval: &mut Evaluator<'v, '_, '_>,
    ) -> anyhow::Result<build::Build> {
        let build_events = partition_build_events(build_events);
        let execution_log = match execution_log {
            Either::Left(b) => (b, vec![]),
            Either::Right(sinks) => (true, sinks.items),
        };
        let (mut resolved_flags, resolved_startup_flags) =
            resolve_invocation_flags(this, "test", rc, &flags.items)?;
        resolved_flags.extend(aspect::materialize(
            &aspects.items,
            &resolved_startup_flags,
        )?);
        let env = Env::from_eval(eval)?;
        let (stdout, stderr) = resolve_stdio(stdio, stdout, stderr)?;
        let test = build::Build::spawn(
            "test",
            targets.items.iter().map(|f| f.as_str().to_string()),
            build_events,
            execution_log,
            workspace_events,
            resolved_flags,
            resolved_startup_flags,
            stdout,
            stderr,
            directory.into_option(),
            build::AnnounceSpawn {
                version: announce_version,
                command: announce_command,
            },
            env.rt.clone(),
        )?;
        Ok(test)
    }

    /// Run `bazel query <expr>` and return the matching targets as an iterable
    /// `Query` (supports `len()`, indexing, and `for t in result`).
    ///
    /// Blocking. Fails with Bazel's own stderr if the query exits non-zero (bad
    /// expression, BUILD error, …) rather than returning an empty set.
    ///
    /// # Arguments
    /// * `expr` - The query expression (e.g. `"deps(//foo)"`, `"kind(cc_*, //...)"`).
    /// * `rc` - `RunCommand` to resolve under; falls back to the active one
    ///   (`use_rc`). With an rc in effect the query self-expands for the
    ///   `query` command (carrying `--ignore_all_rc_files`) so it resolves
    ///   external repos the same way the build does. Without one, Bazel reads
    ///   its own `.bazelrc`.
    /// * `flags` - Per-call command extras appended after the rc expansion.
    ///   Same `str | (str, version-constraint)` shape as `build` / `test`.
    /// * `directory` - Working directory for the query; selects the
    ///   workspace / server. Used for git-worktree-scoped discovery.
    /// * `announce_version` / `announce_command` - Mirror the build/test
    ///   spawn disclosure.
    ///
    /// **Example**
    ///
    /// ```python
    /// for t in ctx.bazel.query("deps(//myapp:main)", rc = rc):
    ///     print(t.name)
    /// ```
    fn query<'v>(
        this: values::Value<'v>,
        #[starlark(require = pos)] expr: String,
        #[starlark(require = named, default = NoneOr::None)] rc: NoneOr<values::Value<'v>>,
        #[starlark(require = named, default = UnpackList::default())] flags: UnpackList<
            Either<values::StringValue<'v>, (values::StringValue<'v>, values::StringValue<'v>)>,
        >,
        #[starlark(require = named, default = NoneOr::None)] directory: NoneOr<String>,
        #[starlark(require = named, default = false)] announce_version: bool,
        #[starlark(require = named, default = false)] announce_command: bool,
    ) -> anyhow::Result<query::Query> {
        let extras = resolve_flags_for_running_bazel(&flags.items)?;
        let (startup, command_flags) = match effective_rc(this, rc) {
            Some(rc) => {
                let (startup, mut base) = rc.resolve_for_command("query")?;
                base.extend(extras);
                (startup, base)
            }
            None => (read_startup_flags(this)?, extras),
        };
        query::run(
            &expr,
            &startup,
            &command_flags,
            directory.into_option(),
            build::AnnounceSpawn {
                version: announce_version,
                command: announce_command,
            },
        )
    }

    /// Run `bazel info` and return all key/value pairs as a dict.
    ///
    /// Blocks until the command completes. Raises an error if Bazel exits
    /// with a non-zero code.
    ///
    /// # Arguments
    /// * `directory`: working directory to run `bazel info` in; selects the
    ///   workspace / server (default: the parent process cwd).
    ///
    /// **Examples**
    ///
    /// ```python
    /// def _show_info_impl(ctx):
    ///     info = ctx.bazel.info()
    ///     print(info["output_base"])
    ///     print(info["execution_root"])
    /// ```
    fn info<'v>(
        this: values::Value<'v>,
        #[starlark(require = named, default = NoneOr::None)] directory: NoneOr<String>,
    ) -> anyhow::Result<SmallMap<String, String>> {
        let startup_flags = read_startup_flags(this)?;

        let mut cmd = bazel_command();
        cmd.args(&startup_flags);
        cmd.arg("info");
        if let Some(dir) = directory.into_option() {
            cmd.current_dir(dir);
        }
        cmd.stdout(Stdio::piped());
        cmd.stderr(Stdio::null());
        cmd.stdin(Stdio::null());
        // Register with the live-bazel registry so OS-signal cancellation
        // can reach this `bazel info` even if the daemon is busy.
        let (child, _guard) = live::spawn_registered(&mut cmd)
            .map_err(|e| anyhow::anyhow!("failed to spawn bazel: {}", e))?;
        let output = child
            .wait_with_output()
            .map_err(|e| anyhow::anyhow!("failed to wait on bazel: {}", e))?;

        if !output.status.success() {
            anyhow::bail!(
                "bazel info failed with exit code {:?}",
                output.status.code()
            );
        }

        let stdout = String::from_utf8_lossy(&output.stdout);
        let mut map = SmallMap::new();
        for line in stdout.lines() {
            if let Some((key, value)) = line.split_once(": ") {
                map.insert(key.trim().to_string(), value.trim().to_string());
            }
        }
        Ok(map)
    }

    /// Shut down the Bazel server for the active run command (`bazel shutdown`),
    /// returning the exit code. Best-effort: a missing or already-stopped server
    /// is not an error, so callers can run this before mutating the output base
    /// (e.g. wiping it) without first checking whether a server is live.
    ///
    /// Goes through the same launcher resolution as `build` / `info` /
    /// `health_check` (honors `BAZEL_REAL` and the `tools/bazel` wrapper via
    /// `bazel_command`, sets `ASPECT_CLI_RUNNING`) and replays the active run
    /// command's startup flags (`--output_base`, …), so it targets exactly the
    /// server those calls started — not whatever a bare `bazel` on PATH would.
    ///
    /// **Examples**
    ///
    /// ```python
    /// def _impl(ctx):
    ///     ctx.bazel.health_check()   # may start a server in the runner output base
    ///     ctx.bazel.shutdown()       # stop it before wiping that output base
    /// ```
    fn shutdown<'v>(this: values::Value<'v>) -> anyhow::Result<i32> {
        let startup_flags = read_startup_flags(this)?;

        let mut cmd = bazel_command();
        cmd.args(&startup_flags);
        cmd.arg("shutdown");
        cmd.stdin(Stdio::null());
        let (mut child, _guard) = live::spawn_registered(&mut cmd)
            .map_err(|e| anyhow::anyhow!("failed to spawn bazel: {}", e))?;
        let status = child
            .wait()
            .map_err(|e| anyhow::anyhow!("failed to wait on bazel: {}", e))?;
        Ok(status.code().unwrap_or(-1))
    }

    /// The Bazel release version as a `str` (e.g. `"7.4.1"`), or `None` for a
    /// non-release build (`development version` / `no_version`) or when the
    /// probe fails.
    ///
    /// Probed once per process (via `bazel info release`) and memoized, so
    /// repeated calls — and other callers that need the version — share a single
    /// probe rather than each shelling out to Bazel.
    ///
    /// By default the pre-release suffix is dropped (`8.0.0-rc2` → `"8.0.0"`),
    /// which is what version-gating wants — a release candidate should match the
    /// same constraints its release will. Pass `strip = False` for the exact
    /// reported version (`"8.0.0-rc2"`) when you need to distinguish an rc.
    ///
    /// **Examples**
    ///
    /// ```python
    /// def _impl(ctx):
    ///     v = ctx.bazel.version()             # "8.0.0"  (rc suffix dropped)
    ///     exact = ctx.bazel.version(strip = False)  # "8.0.0-rc2"
    ///     if v == None:
    ///         print("non-release or unknown Bazel")
    /// ```
    fn version<'v>(
        #[allow(unused)] this: values::Value<'v>,
        #[starlark(require = named, default = true)] strip: bool,
    ) -> anyhow::Result<NoneOr<String>> {
        Ok(match info::release_version() {
            Some(v) if strip => NoneOr::Other(format!("{}.{}.{}", v.major, v.minor, v.patch)),
            Some(v) => NoneOr::Other(v.to_string()),
            None => NoneOr::None,
        })
    }

    /// Probe the Bazel server to determine whether it is responsive.
    ///
    /// Runs `bazel --noblock_for_lock info server_pid`. If the server is
    /// unresponsive, attempts recovery by killing the server process and
    /// re-checking.
    ///
    /// Returns a `HealthCheckResult` with `.success`, `.healthy`, `.message`,
    /// and `.exit_code` attributes.
    ///
    /// **Examples**
    ///
    /// ```python
    /// def _health_probe_impl(ctx):
    ///     result = ctx.bazel.health_check()
    ///     if not result.healthy:
    ///         fail("Bazel server is unhealthy")
    /// ```
    fn health_check<'v>(
        this: values::Value<'v>,
    ) -> anyhow::Result<health_check::HealthCheckResult> {
        let startup_flags = read_startup_flags(this)?;
        Ok(health_check::run(&startup_flags))
    }

    /// Detect and best-effort repair runner-poisoning sandbox state
    /// described by bazelbuild/bazel#23880.
    ///
    /// Inspects `<output_base>/sandbox/` for entries outside Bazel's
    /// `SANDBOX_BASE_PERSISTENT_DIRS` whitelist
    /// (`{.DS_Store, sandbox_stash, sandbox_stash_temp, _moved_trash_dir}`).
    /// Anything else present after a bazel command exited is the
    /// poisoning signature: Bazel's own `afterCommand` cleanup left
    /// state behind because its spawn-runner registry didn't include
    /// the strategy that owned that subtree (see `LinuxSandboxedStrategy.
    /// create` IOException path; fixed upstream by `abe8d6090` in 9.0+).
    ///
    /// When poisoning is detected, every offending entry is
    /// `rm -rf`'d. The result distinguishes three cases:
    ///
    /// - `"clean"`: nothing to remove. Either the sandbox dir didn't
    ///   exist or contained only whitelisted entries.
    /// - `"repaired"`: at least one non-whitelisted entry was found
    ///   and all of them were successfully removed. The runner is
    ///   safe to keep serving jobs.
    /// - `"still_poisoned"`: at least one entry survived the removal
    ///   attempt (e.g. EPERM on the underlying filesystem). The runner
    ///   will keep crashing every bazel command on the same output
    ///   base — the caller should mark it unhealthy.
    ///
    /// Reads `--output_base=` from `ctx.bazel.startup_flags`. When
    /// `--output_base` is not present (e.g. local dev), returns `"clean"`
    /// — there's no way to know which output_base to inspect, and the
    /// failure mode is a Workflows-runner-only concern in practice.
    ///
    /// Safe to call only AFTER the most recent bazel client invocation
    /// has fully exited — a live `bazel build/test` against the same
    /// output_base would race the removal.
    ///
    /// **Examples**
    ///
    /// ```python
    /// def _post_bazel_hook(ctx, exit_code):
    ///     if exit_code == 0:
    ///         return
    ///     r = ctx.bazel.recover_poisoned_sandbox()
    ///     if r.outcome == "repaired":
    ///         print("Recovered from bazel#23880 poisoning: " + ", ".join(r.removed))
    ///     elif r.outcome == "still_poisoned":
    ///         signal_instance_unhealthy()
    /// ```
    fn recover_poisoned_sandbox<'v>(
        this: values::Value<'v>,
    ) -> anyhow::Result<sandbox_recovery::SandboxRecoveryResult> {
        let startup_flags = read_startup_flags(this)?;
        let Some(output_base) = sandbox_recovery::output_base_from_flags(&startup_flags) else {
            return Ok(sandbox_recovery::SandboxRecoveryResult::skipped());
        };
        let outcome = sandbox_recovery::recover(&output_base);
        Ok(sandbox_recovery::SandboxRecoveryResult::from_outcome(
            outcome,
        ))
    }

    /// Parse `.bazelrc` files rooted at `root` and return a `RunCommand`.
    ///
    /// # Arguments
    /// * `root` - Bazel workspace root directory. Defaults to
    ///   `env.bazel_root_dir` — the deepest `MODULE.bazel` / `WORKSPACE`
    ///   ancestor of `cwd`. Pass an explicit `root` only to read a
    ///   bazelrc outside the surrounding workspace; passing the Aspect
    ///   root here in a sub-workspace layout would read the outer
    ///   `.bazelrc` and leak the parent project's flags.
    /// * `startup_flags` - Startup flags (e.g. `["--bazelrc=/path/to/extra.bazelrc"]`).
    /// * `flags` - Command flags to inject as synthetic `always` options; each
    ///   element is a `str` or a `(flag, version_constraint)` tuple.
    /// * `skip_config_if_missing` - `--config` names to drop if undefined.
    /// * `version` - Bazel version for evaluating version-gated options. When
    ///   unset, the running Bazel is probed (only if a gated option exists).
    ///
    /// **Examples**
    ///
    /// ```python
    /// def _impl(ctx):
    ///     rc = ctx.bazel.parse_rc(flags = ["--config=opt"])
    ///     ctx.bazel.use_rc(rc)
    ///     ctx.bazel.build("//...").wait()
    /// ```
    fn parse_rc<'v>(
        #[allow(unused)] this: values::Value<'v>,
        #[starlark(require = named, default = NoneOr::None)] root: NoneOr<String>,
        #[starlark(require = named, default = UnpackList::default())] startup_flags: UnpackList<
            String,
        >,
        #[starlark(require = named, default = UnpackList::default())] flags: UnpackList<
            bazelrc::RcOption,
        >,
        #[starlark(require = named, default = UnpackList::default())]
        skip_config_if_missing: UnpackList<String>,
        #[starlark(require = named, default = NoneOr::None)] version: NoneOr<String>,
        eval: &mut Evaluator<'v, '_, '_>,
    ) -> anyhow::Result<bazelrc::BazelRC> {
        let env = Env::from_eval(eval)?;
        let root = root
            .into_option()
            .map(std::path::PathBuf::from)
            .unwrap_or_else(|| env.bazel_root_dir.clone());
        let rc = bazelrc::BazelRC::new(root, &startup_flags.items, &flags.items)
            .map_err(|e| anyhow::anyhow!("{}", e))?
            .with_skip_config_if_missing(skip_config_if_missing.items);
        let version = resolve_rc_version(version.into_option(), &rc)?;
        Ok(rc.with_version(version))
    }

    /// Create a blank `RunCommand` that reads no `.bazelrc` on disk — only the
    /// flags you provide. Compose further with `merge`.
    ///
    /// # Arguments
    /// * `startup_flags` - Startup flags carried by the run command.
    /// * `flags` - Command flags, same `str | (str, str)` shape as `parse_rc`.
    /// * `version` - Bazel version for evaluating version-gated options.
    fn new_rc<'v>(
        #[allow(unused)] this: values::Value<'v>,
        #[starlark(require = named, default = UnpackList::default())] startup_flags: UnpackList<
            String,
        >,
        #[starlark(require = named, default = UnpackList::default())] flags: UnpackList<
            bazelrc::RcOption,
        >,
        #[starlark(require = named, default = NoneOr::None)] version: NoneOr<String>,
    ) -> anyhow::Result<bazelrc::BazelRC> {
        let rc = bazelrc::BazelRC::blank(&flags.items)
            .with_startup_flags(startup_flags.items)
            .with_version(
                version
                    .into_option()
                    .and_then(|s| semver::Version::parse(&s).ok()),
            );
        Ok(rc)
    }

    /// Render the `--announce-bazel-rc` disclosure for a `RunCommand`: the
    /// options loaded for `command`, grouped by source file, with secrets in
    /// flag values redacted. Replaces the former `rc.announce` method (which
    /// would need workspace-root + redaction context the value can't carry).
    fn announce_rc<'v>(
        #[allow(unused)] this: values::Value<'v>,
        #[starlark(require = pos)] rc: &bazelrc::BazelRC,
        #[starlark(require = named)] command: String,
        #[starlark(require = named, default = false)] ansi: bool,
        #[starlark(require = named, default = 120)] max_width: i64,
        eval: &mut Evaluator<'v, '_, '_>,
    ) -> anyhow::Result<String> {
        let root = Env::from_eval(eval)?.bazel_root_dir.clone();
        let home = std::env::home_dir();
        let redact = stream::redaction::redactor(rc.all_option_values());
        Ok(rc.announce(
            &command,
            ansi,
            max_width.max(0) as usize,
            &root,
            home.as_deref(),
            redact,
        ))
    }

    /// Cancel whatever invocation is currently running on the Bazel server.
    ///
    /// Finds the bazel client process holding the server lock and sends it
    /// SIGINT (graceful cancellation, like Ctrl+C). The client then forwards
    /// a CancelRequest RPC to the server. Returns a `Cancellation` with
    /// status and control methods.
    ///
    /// # Arguments
    /// * `force_kill_after_ms` - If the build is still running after this many
    ///   milliseconds, `wait()` will automatically escalate by sending the 2nd
    ///   and 3rd SIGINT to the Bazel client (the 3rd triggers Bazel's built-in
    ///   server kill, equivalent to Ctrl+C three times). If the client still
    ///   doesn't exit, falls back to SIGKILL on both client and server.
    ///   Defaults to 5000ms. Set to 0 to disable auto-escalation and manage
    ///   cancellation manually via `wait(timeout_ms=...)` and `force()`.
    fn cancel_invocation<'v>(
        this: values::Value<'v>,
        #[starlark(require = named, default = 5000)] force_kill_after_ms: i32,
    ) -> anyhow::Result<cancel::Cancellation> {
        let all_flags = read_startup_flags(this)?;
        let force_kill_after_ms = force_kill_after_ms.max(0) as u64;

        // Send SIGINT to the Bazel client holding the server lock.
        // client_pid() uses --noblock_for_lock so it returns immediately.
        if let Some(pid) = info::client_pid(&all_flags) {
            process::sigint(pid);
        }

        Ok(cancel::Cancellation::new(all_flags, force_kill_after_ms))
    }
}

#[starlark_module]
fn register_build_events(globals: &mut GlobalsBuilder) {
    /// Forward Build Event Protocol events to a gRPC backend.
    ///
    /// Mirrors Bazel's `BuildEventServiceUploader`: transient transport or
    /// gRPC errors trigger a bounded reconnect with full-jitter exponential
    /// backoff, replaying buffered events under their original sequence
    /// numbers. The BES protocol's per-stream dedup makes replay safe.
    ///
    /// # Arguments
    /// * `uri` - BES endpoint. `grpcs://` is rewritten to `https://`.
    /// * `metadata` - Headers attached to every request.
    /// * `max_retries` - Max reconnect attempts after an error before giving
    ///   up (default `4`). `0` disables retry; the sink is still non-fatal.
    /// * `retry_min_delay` - Base delay for exponential backoff
    ///   (default `"1s"`).
    /// * `retry_max_buffer_size` - Cap on the in-flight unacked retry buffer
    ///   (default `10000`). Exceeding it mid-stream is terminal.
    /// * `timeout` - Overall upload deadline (default `"0s"` = no deadline).
    #[starlark(as_type = build::BuildEventSink)]
    fn grpc(
        #[starlark(require = named)] uri: String,
        #[starlark(require = named, default = UnpackDictEntries::default())]
        metadata: UnpackDictEntries<String, String>,
        #[starlark(require = named, default = 4)] max_retries: i32,
        #[starlark(require = named, default = "1s")] retry_min_delay: &str,
        #[starlark(require = named, default = 10_000)] retry_max_buffer_size: i32,
        #[starlark(require = named, default = "0s")] timeout: &str,
    ) -> anyhow::Result<build::BuildEventSink> {
        if max_retries < 0 {
            anyhow::bail!("max_retries must be >= 0, got {max_retries}");
        }
        if retry_max_buffer_size <= 0 {
            anyhow::bail!("retry_max_buffer_size must be > 0, got {retry_max_buffer_size}");
        }
        let retry_min_delay = sink::retry::parse_duration(retry_min_delay)
            .map_err(|e| anyhow::anyhow!("retry_min_delay: {e}"))?;
        let timeout_dur =
            sink::retry::parse_duration(timeout).map_err(|e| anyhow::anyhow!("timeout: {e}"))?;
        let timeout = if timeout_dur.is_zero() {
            None
        } else {
            Some(timeout_dur)
        };
        Ok(build::BuildEventSink::new_grpc(
            uri.replace("grpcs://", "https://"),
            HashMap::from_iter(metadata.entries),
            sink::retry::RetryConfig {
                max_retries: max_retries as u32,
                retry_min_delay,
                retry_max_buffer_size: retry_max_buffer_size as usize,
                timeout,
            },
        ))
    }

    fn file(#[starlark(require = named)] path: String) -> anyhow::Result<build::BuildEventSink> {
        Ok(build::BuildEventSink::new_file(path))
    }

    /// Create a handle iterating this build's BES stream. Pass it in
    /// `build_events=[...]` and then `for event in iter:`. Optional
    /// `kinds=[build_event.TargetCompleted, "named_set_of_files", ...]`
    /// filters at iteration time.
    #[starlark(as_type = build::BuildEventIter)]
    fn iterator(
        #[starlark(require = named, default = NoneOr::None)] kinds: NoneOr<
            UnpackList<values::Value>,
        >,
    ) -> anyhow::Result<build::BuildEventIter> {
        let kinds = match kinds {
            NoneOr::None => None,
            NoneOr::Other(list) => {
                if list.items.is_empty() {
                    anyhow::bail!(
                        "kinds=[] is not valid; omit `kinds` to receive every event kind"
                    );
                }
                let mut set = std::collections::HashSet::new();
                for item in &list.items {
                    set.insert(parse_event_kind(*item)?);
                }
                Some(set)
            }
        };
        Ok(build::BuildEventIter::new(kinds))
    }
}

fn parse_event_kind<'v>(value: values::Value<'v>) -> anyhow::Result<i32> {
    if let Some(n) = value.unpack_i32() {
        return Ok(n);
    }
    if let Some(s) = value.unpack_str() {
        return match s {
            "progress" => Ok(3),
            "aborted" => Ok(4),
            "started" | "build_started" => Ok(5),
            "expanded" | "pattern_expanded" => Ok(6),
            "configured" | "target_configured" => Ok(7),
            "action" | "action_completed" => Ok(8),
            "completed" | "target_completed" => Ok(9),
            "test_result" => Ok(10),
            "finished" | "build_finished" => Ok(11),
            "unstructured_command_line" => Ok(12),
            "structured_command_line" => Ok(13),
            "options_parsed" => Ok(14),
            "named_set_of_files" | "named_set" => Ok(15),
            "workspace_status" => Ok(16),
            "fetch" => Ok(17),
            "configuration" => Ok(19),
            "test_summary" => Ok(20),
            "build_tool_logs" => Ok(21),
            "build_metrics" => Ok(22),
            "build_metadata" => Ok(24),
            "workspace_info" => Ok(25),
            "target_summary" => Ok(26),
            "convenience_symlinks_identified" => Ok(27),
            "exec_request" => Ok(28),
            other => anyhow::bail!(
                "unknown build_event kind '{other}'; pass a `bazel.build.build_event.*` \
                 constant or one of the documented string aliases",
            ),
        };
    }
    anyhow::bail!(
        "kinds entry must be a `bazel.build.build_event.*` constant or a string alias; got {}",
        value.get_type()
    )
}

#[starlark_module]
fn register_execlog_sinks(globals: &mut GlobalsBuilder) {
    #[starlark(as_type = sink::execlog::ExecLogSink)]
    fn file(
        #[starlark(require = named)] path: String,
    ) -> anyhow::Result<sink::execlog::ExecLogSink> {
        Ok(sink::execlog::ExecLogSink::File { path })
    }

    fn compact_file(
        #[starlark(require = named)] path: String,
    ) -> anyhow::Result<sink::execlog::ExecLogSink> {
        Ok(sink::execlog::ExecLogSink::CompactFile { path })
    }
}

#[starlark_module]
fn register_build_types(globals: &mut GlobalsBuilder) {
    const Build: StarlarkValueAsType<build::Build> = StarlarkValueAsType::new();
    const BuildEventIter: StarlarkValueAsType<build::BuildEventIter> = StarlarkValueAsType::new();
    const BuildEventSink: StarlarkValueAsType<build::BuildEventSink> = StarlarkValueAsType::new();
    const BuildStatus: StarlarkValueAsType<build::BuildStatus> = StarlarkValueAsType::new();
    const Cancellation: StarlarkValueAsType<cancel::Cancellation> = StarlarkValueAsType::new();
    const ExecutionLogIterator: StarlarkValueAsType<iter::ExecutionLogIterator> =
        StarlarkValueAsType::new();
    const WorkspaceEventIterator: StarlarkValueAsType<iter::WorkspaceEventIterator> =
        StarlarkValueAsType::new();
}

#[starlark_module]
fn register_execlog_types(globals: &mut GlobalsBuilder) {
    const ExecLogSink: StarlarkValueAsType<sink::execlog::ExecLogSink> = StarlarkValueAsType::new();
}

#[starlark_module]
fn register_query_types(globals: &mut GlobalsBuilder) {
    const Query: StarlarkValueAsType<query::Query> = StarlarkValueAsType::new();
}

#[starlark_module]
fn register_types(globals: &mut GlobalsBuilder) {
    const Bazel: StarlarkValueAsType<FrozenBazel> = StarlarkValueAsType::new();
    const RunCommand: StarlarkValueAsType<bazelrc::BazelRC> = StarlarkValueAsType::new();
    const HealthCheckResult: StarlarkValueAsType<health_check::HealthCheckResult> =
        StarlarkValueAsType::new();
    const SandboxRecoveryResult: StarlarkValueAsType<sandbox_recovery::SandboxRecoveryResult> =
        StarlarkValueAsType::new();
}

pub fn register_globals(globals: &mut GlobalsBuilder) {
    register_types(globals);

    globals.namespace("query", |globals| {
        register_query_types(globals);
        axl_proto::blaze_query_toplevels(globals);
    });

    globals.namespace("build", |globals| {
        register_build_types(globals);
        globals.namespace("build_event", axl_proto::build_event_stream_toplevels);
        globals.namespace("execution_log", axl_proto::tools_protos_toplevels);
        globals.namespace("workspace_event", axl_proto::workspace_log_toplevels);
    });

    globals.namespace("build_events", |globals| {
        register_build_events(globals);
    });

    globals.namespace("execution_log", |globals| {
        register_execlog_types(globals);
        register_execlog_sinks(globals);
    });
}

#[cfg(test)]
mod tests {
    use super::constraint_matches;

    fn version(s: &str) -> semver::Version {
        semver::Version::parse(s).unwrap()
    }

    #[test]
    fn matches_against_a_known_version() {
        let v = version("8.5.1");
        assert!(constraint_matches(">=8", Some(&v)).unwrap());
        assert!(constraint_matches(">=8, <9", Some(&v)).unwrap());
        assert!(!constraint_matches(">=9", Some(&v)).unwrap());
        assert!(!constraint_matches("<7", Some(&v)).unwrap());
    }

    #[test]
    fn unknown_version_assumes_latest() {
        // A non-release build is newer than any release: lower-bound gates
        // match, upper-bound gates and old-major ranges don't.
        assert!(constraint_matches(">=8", None).unwrap());
        assert!(constraint_matches(">=9", None).unwrap());
        assert!(!constraint_matches("<7", None).unwrap());
        assert!(!constraint_matches(">=8, <9", None).unwrap());
    }

    #[test]
    fn pre_release_matches_its_release_constraints() {
        // A release candidate is gated as its eventual release: the pre-release
        // suffix is ignored, so `8.0.0-rc2` satisfies `>=8` (plain semver
        // VersionReq would NOT match a bare pre-release against `>=8`).
        let rc = version("8.0.0-rc2");
        assert!(constraint_matches(">=8", Some(&rc)).unwrap());
        assert!(constraint_matches(">=8.0.0", Some(&rc)).unwrap());
        assert!(!constraint_matches("<8", Some(&rc)).unwrap());
        assert!(!constraint_matches("<7", Some(&rc)).unwrap());
    }

    #[test]
    fn invalid_constraint_is_an_error() {
        let err = constraint_matches("not-a-constraint", None).unwrap_err();
        assert!(err.to_string().contains("invalid version constraint"));
    }
}
