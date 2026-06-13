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
use starlark::values::UnpackValue;
use starlark::values::ValueLike;
use starlark::values::dict::UnpackDictEntries;
use starlark::values::list::ListRef;
use starlark::values::list::UnpackList;
use starlark::values::none::NoneOr;
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

mod build;
mod cancel;
mod capture;
mod health_check;
mod info;
mod iter;
pub mod live;
mod process;
mod query;
mod rc;
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
fn constraint_matches(constraint: &str, version: Option<&semver::Version>) -> anyhow::Result<bool> {
    let req = semver::VersionReq::parse(constraint)
        .map_err(|e| anyhow::anyhow!("invalid version constraint '{}': {}", constraint, e))?;
    let assumed_latest = semver::Version::new(u64::MAX, u64::MAX, u64::MAX);
    Ok(req.matches(version.unwrap_or(&assumed_latest)))
}

#[derive(Debug, Display, ProvidesStaticType, Trace, NoSerialize, Allocative)]
#[display("<bazel.Bazel>")]
pub struct Bazel<'v> {
    pub startup_flags: values::Value<'v>,
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
            startup_flags: self.startup_flags.freeze(freezer)?,
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
    pub startup_flags: values::FrozenValue,
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

fn read_startup_flags<'v>(this: values::Value<'v>) -> anyhow::Result<Vec<String>> {
    let flags_val = if let Some(b) = this.downcast_ref::<Bazel>() {
        b.startup_flags
    } else if let Some(b) = this.downcast_ref::<FrozenBazel>() {
        b.startup_flags.to_value()
    } else {
        return Ok(vec![]);
    };
    let list = ListRef::from_value(flags_val)
        .ok_or_else(|| anyhow::anyhow!("startup_flags is not a list"))?;
    list.iter()
        .enumerate()
        .map(|(i, v)| {
            v.unpack_str().map(str::to_string).ok_or_else(|| {
                anyhow::anyhow!("startup_flags[{}]: expected str, got {}", i, v.get_type())
            })
        })
        .collect()
}

#[starlark_module]
pub(crate) fn bazel_methods(registry: &mut MethodsBuilder) {
    /// Mutable list of startup flags prepended to every Bazel invocation on this context.
    #[starlark(attribute)]
    fn startup_flags<'v>(this: values::Value<'v>) -> anyhow::Result<values::Value<'v>> {
        if let Some(b) = this.downcast_ref::<Bazel>() {
            Ok(b.startup_flags)
        } else if let Some(b) = this.downcast_ref::<FrozenBazel>() {
            Ok(b.startup_flags.to_value())
        } else {
            Err(anyhow::anyhow!("expected Bazel"))
        }
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
    /// * `output` - An `OutputProcessor` from `bazel.output.processor(...)`.
    ///   When set, the child's stderr is captured by the runtime, run through
    ///   the output processing pipeline, and forwarded to the real stderr
    ///   (overriding the resolved `stderr` slot). Omit (the default) to leave
    ///   stderr handling to `stderr`/`stdio`/inherit.
    /// * `current_dir` - Working directory for the Bazel invocation.
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
        #[starlark(require = named)] stdout: Option<NoneOr<Writable>>,
        #[starlark(require = named)] stderr: Option<NoneOr<Writable>>,
        #[starlark(require = named, default = NoneOr::None)] stdio: NoneOr<StdStdio>,
        #[starlark(require = named, default = NoneOr::None)] output: NoneOr<build::OutputProcessor>,
        #[starlark(require = named, default = NoneOr::None)] current_dir: NoneOr<String>,
        #[starlark(require = named, default = false)] announce_version: bool,
        #[starlark(require = named, default = false)] announce_command: bool,
        eval: &mut Evaluator<'v, '_, '_>,
    ) -> anyhow::Result<build::Build> {
        let build_events = partition_build_events(build_events);
        let execution_log = match execution_log {
            Either::Left(b) => (b, vec![]),
            Either::Right(sinks) => (true, sinks.items),
        };
        let resolved_flags = resolve_flags_for_running_bazel(&flags.items)?;
        let resolved_startup_flags = read_startup_flags(this)?;
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
            output.into_option(),
            current_dir.into_option(),
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
    /// * `output` - An `OutputProcessor` from `bazel.output.processor(...)`.
    ///   When set, the child's stderr is captured by the runtime, run through
    ///   the output processing pipeline, and forwarded to the real stderr
    ///   (overriding the resolved `stderr` slot). Omit (the default) to leave
    ///   stderr handling to `stderr`/`stdio`/inherit.
    /// * `current_dir` - Working directory for the Bazel invocation.
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
        #[starlark(require = named)] stdout: Option<NoneOr<Writable>>,
        #[starlark(require = named)] stderr: Option<NoneOr<Writable>>,
        #[starlark(require = named, default = NoneOr::None)] stdio: NoneOr<StdStdio>,
        #[starlark(require = named, default = NoneOr::None)] output: NoneOr<build::OutputProcessor>,
        #[starlark(require = named, default = NoneOr::None)] current_dir: NoneOr<String>,
        #[starlark(require = named, default = false)] announce_version: bool,
        #[starlark(require = named, default = false)] announce_command: bool,
        eval: &mut Evaluator<'v, '_, '_>,
    ) -> anyhow::Result<build::Build> {
        let build_events = partition_build_events(build_events);
        let execution_log = match execution_log {
            Either::Left(b) => (b, vec![]),
            Either::Right(sinks) => (true, sinks.items),
        };
        let resolved_flags = resolve_flags_for_running_bazel(&flags.items)?;
        let resolved_startup_flags = read_startup_flags(this)?;
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
            output.into_option(),
            current_dir.into_option(),
            build::AnnounceSpawn {
                version: announce_version,
                command: announce_command,
            },
            env.rt.clone(),
        )?;
        Ok(test)
    }

    /// The query system provides a programmatic interface for analyzing build dependencies
    /// and target relationships. Queries are constructed using a chain API and are lazily
    /// evaluated only when `.eval()` is explicitly called.
    ///
    /// The entry point is `ctx.bazel.query()`, which returns a `query` for creating initial
    /// query expressions. Most operations operate on `query` objects, which represent
    /// sets of targets that can be filtered, transformed, and combined.
    ///
    /// **Example**
    ///
    /// ```starlark
    /// **Query** dependencies of a target
    /// deps = ctx.bazel.query().targets("//myapp:main").deps()
    /// all_deps = deps.eval()
    ///
    /// **Chain** multiple operations
    /// sources = ctx.bazel.query().targets("//myapp:main")
    ///     .deps()
    ///     .kind("source file")
    ///     .eval()
    /// ```
    fn query<'v>(this: values::Value<'v>) -> anyhow::Result<query::Query> {
        let startup_flags = read_startup_flags(this)?;
        Ok(query::Query::new(startup_flags))
    }

    /// Run `bazel info` and return all key/value pairs as a dict.
    ///
    /// Blocks until the command completes. Raises an error if Bazel exits
    /// with a non-zero code.
    ///
    /// # Arguments
    /// * `workdir`: workspace root to run `bazel info` in (default: inferred from ctx)
    ///
    /// **Examples**
    ///
    /// ```python
    /// def _show_info_impl(ctx):
    ///     info = ctx.bazel.info()
    ///     print(info["output_base"])
    ///     print(info["execution_root"])
    /// ```
    fn info<'v>(this: values::Value<'v>) -> anyhow::Result<SmallMap<String, String>> {
        let startup_flags = read_startup_flags(this)?;

        let mut cmd = bazel_command();
        cmd.args(&startup_flags);
        cmd.arg("info");
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

    /// Parse `.bazelrc` files rooted at `root` and return a `BazelRC` object.
    ///
    /// # Arguments
    /// * `root` - Bazel workspace root directory. Defaults to
    ///   `env.bazel_root_dir` — the deepest `MODULE.bazel` / `WORKSPACE`
    ///   ancestor of `cwd`. Pass an explicit `root` only to read a
    ///   bazelrc outside the surrounding workspace; passing the Aspect
    ///   root here in a sub-workspace layout would read the outer
    ///   `.bazelrc` and leak the parent project's flags.
    /// * `startup_flags` - Startup flags (e.g. `["--bazelrc=/path/to/extra.bazelrc"]`).
    /// * `flags` - Command-line flags to inject as synthetic `always` options
    ///   (e.g. `["--config=opt"]`).
    ///
    /// **Examples**
    ///
    /// ```python
    /// def _impl(ctx):
    ///     rc = ctx.bazel.parse_rc(flags = ["--config=opt"])
    ///     build = ctx.bazel.build("//...", flags = rc.expand(command = "build"))
    ///     build.wait()
    /// ```
    fn parse_rc<'v>(
        #[allow(unused)] this: values::Value<'v>,
        #[starlark(require = named, default = NoneOr::None)] root: NoneOr<String>,
        #[starlark(require = named, default = UnpackList::default())] startup_flags: UnpackList<
            values::Value<'v>,
        >,
        #[starlark(require = named, default = UnpackList::default())] flags: UnpackList<
            values::Value<'v>,
        >,
        #[starlark(require = named, default = UnpackList::default())]
        skip_config_if_missing: UnpackList<String>,
        eval: &mut Evaluator<'v, '_, '_>,
    ) -> anyhow::Result<rc::StarlarkBazelRC> {
        fn unpack_rc_option<'v>(item: values::Value<'v>) -> anyhow::Result<bazelrc::RcOption> {
            if let Some(s) = item.unpack_str() {
                return Ok(bazelrc::RcOption {
                    value: s.to_owned(),
                    ..bazelrc::RcOption::default()
                });
            }
            if let Ok(Some(tup)) = UnpackTuple::<&str>::unpack_value(item) {
                if let Some(flag) = tup.items.first() {
                    return Ok(bazelrc::RcOption {
                        value: flag.to_string(),
                        version_condition: tup.items.get(1).map(|s| s.to_string()),
                        ..bazelrc::RcOption::default()
                    });
                }
            }
            Err(anyhow::anyhow!(
                "parse_rc: flag items must be str or (str, str), got {}",
                item.get_type()
            ))
        }

        let env = Env::from_eval(eval)?;
        let root = root
            .into_option()
            .map(std::path::PathBuf::from)
            .unwrap_or_else(|| env.bazel_root_dir.clone());
        let startup_flags_vec: Vec<&str> = startup_flags
            .items
            .iter()
            .map(|v| {
                v.unpack_str().ok_or_else(|| {
                    anyhow::anyhow!(
                        "parse_rc: startup_flags items must be str, got {}",
                        v.get_type()
                    )
                })
            })
            .collect::<anyhow::Result<_>>()?;
        let flags_vec: Vec<bazelrc::RcOption> = flags
            .items
            .iter()
            .map(|v| unpack_rc_option(*v))
            .collect::<anyhow::Result<_>>()?;
        let inner = bazelrc::BazelRC::new(root, &startup_flags_vec, &flags_vec)
            .map_err(|e| anyhow::anyhow!("{}", e))?;
        Ok(rc::StarlarkBazelRC {
            inner,
            skip_config_if_missing: skip_config_if_missing.items,
        })
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
fn register_output(globals: &mut GlobalsBuilder) {
    /// Create a captured-output processor. Pass it as
    /// `ctx.bazel.build(output = bazel.output.processor(...))` to capture the
    /// child's stderr, run it through the processing pipeline, and forward it
    /// to the real stderr instead of letting Bazel write the terminal directly.
    ///
    /// # Arguments
    /// * `tty` - Whether the destination stderr is an interactive terminal.
    ///   `True` → allocate a PTY so Bazel keeps its live curses progress UI
    ///   (bytes forwarded near-verbatim). `False` → a plain pipe, so Bazel
    ///   emits clean newline-terminated lines (set `--curses=no --isatty=0` on
    ///   the invocation to match). Defaults to auto-detecting the real stderr.
    ///
    /// The caller is responsible for setting `--isatty` / `--curses` on the
    /// Bazel invocation to match the chosen mode (see `bazel_runner.axl`).
    #[starlark(as_type = build::OutputProcessor)]
    fn processor(
        #[starlark(require = named, default = NoneOr::None)] tty: NoneOr<bool>,
    ) -> anyhow::Result<build::OutputProcessor> {
        use std::io::IsTerminal;
        let is_tty = match tty {
            NoneOr::Other(b) => b,
            NoneOr::None => std::io::stderr().is_terminal(),
        };
        let mode = if is_tty {
            build::CaptureMode::Pty
        } else {
            build::CaptureMode::Pipe
        };
        Ok(build::OutputProcessor::new(mode))
    }
}

#[starlark_module]
fn register_output_types(globals: &mut GlobalsBuilder) {
    const OutputProcessor: StarlarkValueAsType<build::OutputProcessor> = StarlarkValueAsType::new();
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
    const TargetSet: StarlarkValueAsType<query::TargetSet> = StarlarkValueAsType::new();
}

#[starlark_module]
fn register_types(globals: &mut GlobalsBuilder) {
    const Bazel: StarlarkValueAsType<FrozenBazel> = StarlarkValueAsType::new();
    const BazelRC: StarlarkValueAsType<rc::StarlarkBazelRC> = StarlarkValueAsType::new();
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

    globals.namespace("output", |globals| {
        register_output_types(globals);
        register_output(globals);
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
    fn invalid_constraint_is_an_error() {
        let err = constraint_matches("not-a-constraint", None).unwrap_err();
        assert!(err.to_string().contains("invalid version constraint"));
    }
}
