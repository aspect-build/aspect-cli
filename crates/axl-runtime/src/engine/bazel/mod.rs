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

use crate::engine::store::Env;
use axl_proto;

mod build;
mod cancel;
mod health_check;
mod info;
mod iter;
mod process;
mod query;
mod rc;
mod sink;
mod stream;

/// Resolve which `bazel` binary to spawn. Honors the `BAZEL_REAL` env var
/// (the bazelisk convention) so wrapped invocations and tests can substitute
/// their own binary; falls back to plain `"bazel"` and lets the OS resolve
/// it via `PATH`.
pub(crate) fn bazel_binary() -> String {
    std::env::var("BAZEL_REAL").unwrap_or_else(|_| "bazel".to_string())
}

/// Resolve a mixed list of plain flags and conditional `(flag, constraint)` tuples into
/// a `Vec<String>`. Returns only the flags whose semver constraint matches `version`.
/// When `version` is `None` all items must be plain strings (i.e. `Either::Left`).
fn resolve_flags<'v>(
    items: &[Either<values::StringValue<'v>, (values::StringValue<'v>, values::StringValue<'v>)>],
    version: Option<&semver::Version>,
) -> anyhow::Result<Vec<String>> {
    let mut result = Vec::with_capacity(items.len());
    for item in items {
        match item {
            Either::Left(s) => result.push(s.as_str().to_string()),
            Either::Right((flag, constraint)) => {
                let version =
                    version.expect("server_info must be called when conditional flags are present");
                let req = semver::VersionReq::parse(constraint.as_str()).map_err(|e| {
                    anyhow::anyhow!(
                        "invalid version constraint '{}': {}",
                        constraint.as_str(),
                        e
                    )
                })?;
                if req.matches(version) {
                    result.push(flag.as_str().to_string());
                }
            }
        }
    }
    Ok(result)
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
    /// * `inherit_stdout` - Inherit stdout from the parent process.
    /// * `inherit_stderr` - Inherit stderr from the parent process. Defaults to `True`.
    /// * `current_dir` - Working directory for the Bazel invocation.
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
    /// )
    /// status = build.wait()
    /// ```
    fn build<'v>(
        this: values::Value<'v>,
        #[starlark(args)] targets: UnpackTuple<values::StringValue>,
        #[starlark(require = named, default = Either::Left(false))] build_events: Either<
            bool,
            UnpackList<build::BuildEventSink>,
        >,
        #[starlark(require = named, default = false)] workspace_events: bool,
        #[starlark(require = named, default = Either::Left(false))] execution_log: Either<
            bool,
            UnpackList<sink::execlog::ExecLogSink>,
        >,
        #[starlark(require = named, default = UnpackList::default())] flags: UnpackList<
            Either<values::StringValue<'v>, (values::StringValue<'v>, values::StringValue<'v>)>,
        >,
        #[starlark(require = named, default = false)] inherit_stdout: bool,
        #[starlark(require = named, default = true)] inherit_stderr: bool,
        #[starlark(require = named, default = NoneOr::None)] current_dir: NoneOr<String>,
        eval: &mut Evaluator<'v, '_, '_>,
    ) -> anyhow::Result<build::Build> {
        let build_events = match build_events {
            // `False` → no BEP stream; `True` → open the stream and add
            // a default local sink so the AXL task can iterate via
            // `build.build_events()` (the only point of `True`).
            Either::Left(false) => (false, vec![]),
            Either::Left(true) => (
                true,
                vec![build::BuildEventSink::Local { buffer_cap: 10_000 }],
            ),
            Either::Right(sinks) => (true, sinks.items),
        };
        let execution_log = match execution_log {
            Either::Left(b) => (b, vec![]),
            Either::Right(sinks) => (true, sinks.items),
        };
        let has_conditional = flags.items.iter().any(|f| f.is_right());
        let bazel_version = if has_conditional {
            let (_, version) = info::server_info()
                .map_err(|e| anyhow::anyhow!("failed to get Bazel server info: {}", e))?;
            Some(version)
        } else {
            None
        };
        let resolved_flags = resolve_flags(&flags.items, bazel_version.as_ref())?;
        let resolved_startup_flags = read_startup_flags(this)?;
        let env = Env::from_eval(eval)?;
        let build = build::Build::spawn(
            "build",
            targets.items.iter().map(|f| f.as_str().to_string()),
            build_events,
            execution_log,
            workspace_events,
            resolved_flags,
            resolved_startup_flags,
            inherit_stdout,
            inherit_stderr,
            current_dir.into_option(),
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
    /// * `inherit_stdout` - Inherit stdout from the parent process.
    /// * `inherit_stderr` - Inherit stderr from the parent process. Defaults to `True`.
    /// * `current_dir` - Working directory for the Bazel invocation.
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
            UnpackList<build::BuildEventSink>,
        >,
        #[starlark(require = named, default = false)] workspace_events: bool,
        #[starlark(require = named, default = Either::Left(false))] execution_log: Either<
            bool,
            UnpackList<sink::execlog::ExecLogSink>,
        >,
        #[starlark(require = named, default = UnpackList::default())] flags: UnpackList<
            Either<values::StringValue<'v>, (values::StringValue<'v>, values::StringValue<'v>)>,
        >,
        #[starlark(require = named, default = false)] inherit_stdout: bool,
        #[starlark(require = named, default = true)] inherit_stderr: bool,
        #[starlark(require = named, default = NoneOr::None)] current_dir: NoneOr<String>,
        eval: &mut Evaluator<'v, '_, '_>,
    ) -> anyhow::Result<build::Build> {
        let build_events = match build_events {
            // See the matching arm in `build` above — `True` is sugar
            // for a default local sink.
            Either::Left(false) => (false, vec![]),
            Either::Left(true) => (
                true,
                vec![build::BuildEventSink::Local { buffer_cap: 10_000 }],
            ),
            Either::Right(sinks) => (true, sinks.items),
        };
        let execution_log = match execution_log {
            Either::Left(b) => (b, vec![]),
            Either::Right(sinks) => (true, sinks.items),
        };
        let has_conditional = flags.items.iter().any(|f| f.is_right());
        let bazel_version = if has_conditional {
            let (_, version) = info::server_info()
                .map_err(|e| anyhow::anyhow!("failed to get Bazel server info: {}", e))?;
            Some(version)
        } else {
            None
        };
        let resolved_flags = resolve_flags(&flags.items, bazel_version.as_ref())?;
        let resolved_startup_flags = read_startup_flags(this)?;
        let env = Env::from_eval(eval)?;
        let test = build::Build::spawn(
            "test",
            targets.items.iter().map(|f| f.as_str().to_string()),
            build_events,
            execution_log,
            workspace_events,
            resolved_flags,
            resolved_startup_flags,
            inherit_stdout,
            inherit_stderr,
            current_dir.into_option(),
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
    fn query<'v>(#[allow(unused)] this: values::Value<'v>) -> anyhow::Result<query::Query> {
        Ok(query::Query::new())
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

        let mut cmd = std::process::Command::new(bazel_binary());
        cmd.args(&startup_flags);
        cmd.arg("info");
        cmd.stdout(Stdio::piped());
        cmd.stderr(Stdio::null());
        cmd.stdin(Stdio::null());
        let output = cmd
            .output()
            .map_err(|e| anyhow::anyhow!("failed to spawn bazel: {}", e))?;

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

    /// Cancel whatever invocation is currently running on the Bazel server.
    ///
    /// Finds the bazel client process holding the server lock and sends it
    /// SIGINT (graceful cancellation, like Ctrl+C). The client then forwards
    /// a CancelRequest RPC to the server.
    /// Returns an `Cancellation` with status and control methods.
    /// Parse `.bazelrc` files rooted at `root` and return a `BazelRC` object.
    ///
    /// # Arguments
    /// * `root` - Workspace root directory. Defaults to the workspace root from `ctx`.
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
            .unwrap_or_else(|| env.root_dir.clone());
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
    /// * `error_strategy` - How a terminal failure surfaces. One of
    ///   `"fail_at_end"`, `"warn"` (default), `"ignore"`.
    #[starlark(as_type = build::BuildEventSink)]
    fn grpc(
        #[starlark(require = named)] uri: String,
        #[starlark(require = named, default = UnpackDictEntries::default())]
        metadata: UnpackDictEntries<String, String>,
        #[starlark(require = named, default = 4)] max_retries: i32,
        #[starlark(require = named, default = "1s")] retry_min_delay: &str,
        #[starlark(require = named, default = 10_000)] retry_max_buffer_size: i32,
        #[starlark(require = named, default = "0s")] timeout: &str,
        #[starlark(require = named, default = "warn")] error_strategy: &str,
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
        let error_strategy =
            sink::retry::ErrorStrategy::parse(error_strategy).map_err(|e| anyhow::anyhow!(e))?;
        Ok(build::BuildEventSink::Grpc {
            uri: uri.replace("grpcs://", "https://"),
            metadata: HashMap::from_iter(metadata.entries),
            retry: sink::retry::RetryConfig {
                max_retries: max_retries as u32,
                retry_min_delay,
                retry_max_buffer_size: retry_max_buffer_size as usize,
                timeout,
                error_strategy,
            },
        })
    }

    fn file(#[starlark(require = named)] path: String) -> anyhow::Result<build::BuildEventSink> {
        Ok(build::BuildEventSink::File { path })
    }

    /// Declare the AXL task's intent to subscribe to the BES stream via
    /// `build.build_events()`. The runtime pre-registers a receiver
    /// inside `ctx.bazel.build(...)` — before bazel opens the BEP FIFO
    /// and before remote sinks touch the network — so the early burst
    /// (`build_started`, `target_completed`, `named_set_of_files`) is
    /// buffered for the consumer regardless of how slow the AXL task is
    /// to call `build_events()` afterward.
    ///
    /// `buffer_cap` bounds undrained accumulation. If the consumer
    /// falls behind by more than `buffer_cap` events, the broadcaster
    /// drops the subscription on the next overflow send and the AXL
    /// iterator sees `Disconnected`. Default `10000` is enough headroom
    /// for tasks that emit progress every BES tick without leaking
    /// memory if the consumer is buggy and never starts draining.
    ///
    /// `ctx.bazel.build(build_events = True)` is sugar for
    /// `build_events = [bazel.build_events.local()]`.
    #[starlark(as_type = build::BuildEventSink)]
    fn local(
        #[starlark(require = named, default = 10_000)] buffer_cap: i32,
    ) -> anyhow::Result<build::BuildEventSink> {
        if buffer_cap <= 0 {
            anyhow::bail!("buffer_cap must be > 0, got {buffer_cap}");
        }
        Ok(build::BuildEventSink::Local {
            buffer_cap: buffer_cap as usize,
        })
    }
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
    const BuildEventIterator: StarlarkValueAsType<iter::BuildEventIterator> =
        StarlarkValueAsType::new();
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
