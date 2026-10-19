# RFC: buildx, a four-noun `ctx.bazel`

Status: draft. Owner: sahin@aspect.build. Started 2026-10-14. Proof of concept: `crates/buildx`, a standalone binary that runs a BXL-shaped Starlark script against a Bazel release's engine hosted in its own process, with live handles over the engine's own Starlark values; and `crates/blazon`, which generates the JVM bindings per Bazel version from the server jar.

## Summary

`ctx.bazel` is redesigned around four values, Configuration, Target, Artifact and Build, and the operations between them: `analyze`, `build`, `ensure`, `upload`, `command`, `test`. Nothing about running a target is a flag on a build. Materialization is an explicit step that names its destination. Local and remote execution share one `command` vocabulary. Every blocking handle is an iterator, and `iter.combine` is the only way to wait on several at once.

The API is a frontend for Bazel's engine. It replaces the command layer, the client, the dispatcher, rc files, flags and the console, with tasks in AXL that drive the engine in process through four seams. The section "The frontend" is the plan. Running the same API over a stock `bazel` binary is the eject path, covered in Appendix A, and nothing in it changes a signature.

## Motivation

Every task that spawns a built target today goes through `runnable.axl`, which passes `--build_runfile_links`. That is a configuration option, so for a workspace with `--nobuild_runfile_links` in its rc, alternating `aspect build` and `aspect run` discards Bazel's analysis cache. Verified on Bazel 9.0.1:

```
WARNING: Build option --build_runfile_links has changed, discarding analysis cache
INFO: Analyzed target //tools/gazelle:gazelle_from_source (0 packages loaded, 23023 targets configured)
```

`bazel run` never flips the flag. It builds, then creates the one runfiles tree it needs from `<exe>.runfiles_manifest` as a local post-build step (`RunfilesTreeUpdater`, `SymlinkTreeHelper`). The issue [aspect-build/aspect-cli#1497](https://github.com/aspect-build/aspect-cli/issues/1497) asks for a run primitive that does the same.

Fixing only that would leave `runnable.axl` as the de-facto run API: BES fileset walking, a hand-rolled aspect, download-floor flag parsing and a spawn helper, all in AXL, all reimplemented by each new task. The wider requirements that surfaced while designing the fix are listed below, and they do not fit a `run` verb bolted onto `build`.

## Requirements

1. Build one or many targets in one invocation and act on each the moment it completes.
2. Run a built target locally with the existing `std.process.Command` surface for stdio, cwd, process group and cancellation.
3. Run a built target on the remote execution cluster with extra host files placed at relative paths in the input root.
4. Never toggle a configuration option to do any of this. Runfiles trees come from the manifest. `--build_runfile_manifests` is the one precondition, as it is for `bazel run`.
5. Failures are data per target. Process exit codes are values.
6. Repeated calls in a watch loop are cheap when nothing changed.
7. Bazel's own names where they exist: `DefaultInfo`, `files_to_run`, `RunEnvironmentInfo`, `File.path`, `File.short_path`.
8. No verb that mirrors another verb's parameters. No parameter per requirement. No REAPI vocabulary in AXL.
9. No tick loops. Starlark has no `while`, so multiplexing is an iterator.

## Non-goals

- A general analysis API over arbitrary providers. `analyze` returns the providers Bazel exposes to the CLI plus what the runtime's aspect records.
- Running inside Bazel's action graph. Execution happens outside it, locally or on the cluster.
- Interactive remote execution. REAPI has no stdin.
- Test sharding and coverage orchestration beyond what `bazel test` already does.

## The four nouns

### Configuration

A value made once and passed to every operation, in place of flag lists on every call.

```python
cfg = ctx.bazel.configure(
    rc = ctx.bazel.active_rc(),          # RunCommand | None
    flags = ["-c", "opt"],               # list[str | tuple[str, str]]
    platform = None,                     # str | None, the target platform label
    directory = None,                    # str | None
)
```

`Configuration` is a record. Two configurations with equal fields are the same configuration to Bazel, which is what makes them cheap to reuse across a watch loop.

### Target

A configured target: a label under a configuration, with its providers.

```python
Target = record(
    label = field(str),                              # apparent form as the caller wrote it
    kind = field(str),                               # rule class
    tags = field(list[str]),
    attrs = field(Attrs),                            # args: list[str], expanded and tokenized
    providers = field(Providers),
    platform = field(Platform),                      # label, exec_properties
    test = field(TestInfo | None),
)

Providers = record(
    DefaultInfo = field(DefaultInfo),                # files, files_to_run
    RunEnvironmentInfo = field(RunEnvironmentInfo | None),
    OutputGroupInfo = field(dict[str, list[Artifact]]),
)

DefaultInfo = record(files = field(list[Artifact]), files_to_run = field(FilesToRun | None))
FilesToRun = record(
    executable = field(Artifact),
    runfiles = field(Runfiles),
    runfiles_manifest = field(Artifact | None),
    repo_mapping_manifest = field(Artifact | None),
)
Runfiles = record(
    workspace_name = field(str),
    files = field(list[Artifact]),                   # lazy: see Appendix A
    symlinks = field(dict[str, Artifact]),
    root_symlinks = field(dict[str, Artifact]),
    empty_filenames = field(list[str]),
)
RunEnvironmentInfo = record(environment = field(dict[str, str]), inherited_environment = field(list[str]))
Platform = record(label = field(str), exec_properties = field(dict[str, str]))
TestInfo = record(size = field(str), timeout = field(str), shard_count = field(int), flaky = field(bool))
```

### Artifact

A file with identity. Where its bytes are is a separate question answered by `ensure` and `upload`.

```python
Artifact = record(
    path = field(str),                               # exec-root relative, Bazel's File.path
    short_path = field(str),                         # runfiles relative, Bazel's File.short_path
    owner = field(str | None),
    is_source = field(bool),
    is_directory = field(bool),
    digest = field(str | None),                      # known once built
    length = field(int | None),
)
```

### Build

A handle over one invocation, indexable by target, iterable in completion order.

```python
build = ctx.bazel.build(*what, config = cfg)         # what: Target | Artifact | str, or output groups via Target.providers.OutputGroupInfo[...]

result = build[t]                                    # BuildResult, blocks until t completes or the build ends
for t, result in build: ...                          # completion order, ends when the build ends
status = build.wait()                                # BuildStatus: success, code
build.try_wait(timeout_ms), build.interrupt(), build.cancellation

BuildResult = record(
    target = field(Target),
    success = field(bool),
    artifacts = field(list[Artifact]),               # what was built, with digests
    failures = field(list[Failure]),                 # kind: enum("failed", "aborted", "not_built"), message
)
```

Failures are data. A `BuildResult` with `success = False` can be inspected, logged and skipped without catching anything. The build's own exit is `build.wait()`.

## The operations

### `analyze`

```python
targets = ctx.bazel.analyze("//svc/...", config = cfg)   # Analysis: indexable by label, iterable
t = targets["//svc:api"]
```

Analysis never executes an action. It is how a task learns what a target would run before deciding to build it, and it is the only place providers come from.

### `ensure` and `upload`

Both put bytes where a command will need them. They name different destinations because they have different costs and different failure modes, and a task should say which it meant.

```python
exe = ctx.bazel.ensure(t.providers.DefaultInfo.files_to_run)   # Ensured: bytes on this machine, runfiles tree laid out
up  = ctx.bazel.upload(t.providers.DefaultInfo.files_to_run)   # Uploaded: bytes in the CAS, runfiles as a Merkle tree
```

- `ensure` accepts an `Artifact`, a `FilesToRun`, a `BuildResult` or a list, and preserves the shape of its input. It is idempotent: a runfiles tree whose `MANIFEST` digest matches is left alone. On Windows or under `--noenable_runfiles` it writes only `MANIFEST` and the workspace directory, which is what the native launcher reads. It refuses, with `Precondition`, when the configuration disables runfile manifests.
- `upload` accepts a `FilesToRun`. It returns the executable and every runfile by digest, the target platform's exec properties, and the endpoint it uploaded to. It never reads runfiles from local disk, so it works where nothing is local. It does not touch host files; those belong to the command.

Everything `upload` sends lands in a shared, content-addressed store readable by anyone with access to that instance. The runtime announces every host-sourced input by path before uploading it, and the documentation says so in the first paragraph.

### `command`

One function, two command types, one vocabulary.

```python
ctx.bazel.command(exe) -> std.process.Command        # for an Ensured
ctx.bazel.command(up)  -> bazel.RemoteCommand         # for an Uploaded
```

Both are populated as `bazel run` would populate them: argv is the executable followed by `t.attrs.args`, env is `RunEnvironmentInfo.environment` plus `BUILD_WORKSPACE_DIRECTORY`, `BUILD_WORKING_DIRECTORY` and the runfiles variables, cwd is the runfiles workspace directory. Both have `args`, `env`, `current_dir`, `stdin`, `stdout`, `stderr` and `spawn(cancellation =)`. `RemoteCommand` adds `input(path, host_path)`, `platform(dict)` and `timeout(ms)`, because only the remote side has them. `spawn()` returns `std.process.Child` or `bazel.Execution`; the two share `wait`, `try_wait`, `kill` and `cancellation`, and both yield one exit status when iterated.

A remote `spawn()` always sets `do_not_cache` and skips the cache lookup, since a run is a side effect. It refuses an empty platform rather than scheduling onto an arbitrary pool.

### `test`

```python
for result in ctx.bazel.test(t, config = cfg): ...   # TestResult: status, attempt, log Artifact, xml Artifact
```

Local `test` sets the `TEST_*` environment the way Bazel's test setup does. Remote `test` is what `bazel test` already does on the cluster.

### `iter.combine`

The one multiplexer. Every blocking handle is an iterator: `Build` yields `(Target, BuildResult)` per completion, `Child` and `Execution` yield one exit status, `cancellation.Token` yields once when cancelled, `fs.Watch` yields batches, `sleep_iter` yields ticks for callers that want a cadence.

```python
combined = iter.combine(build, ctx.cancellation.root)
for src, item in combined:                           # blocks until any source has an item
    if src == build:
        t, result = item
        child = ctx.bazel.command(ctx.bazel.ensure(result.target.providers.DefaultInfo.files_to_run)).spawn()
        running[t.label] = child
        combined.add(child)
    elif src == ctx.cancellation.root:
        break
    else:
        label = [l for l, c in running.items() if c == src][0]
        rows.append((label, running.pop(label).code))
```

A source leaves the set when it ends. The combined iterator ends when the set is empty. `try_pop` and `try_wait` remain as non-blocking escape hatches.

## Examples

### `aspect run`

```python
def _impl(ctx: TaskContext) -> int:
    cfg = ctx.bazel.configure(rc = ctx.bazel.active_rc(), flags = ctx.args.bazel_flags)
    t = ctx.bazel.analyze(ctx.args.target, config = cfg)[ctx.args.target]
    if t.providers.DefaultInfo.files_to_run == None:
        return TaskConclusion(exit_code = 1, message = "%s is not runnable" % t.label)
    build = ctx.bazel.build(t, config = cfg)
    result = build[t]
    if not result.success:
        return build.wait().code
    exe = ctx.bazel.ensure(t.providers.DefaultInfo.files_to_run)
    cmd = ctx.bazel.command(exe).args(ctx.args.args)
    if ctx.args.run_in:
        cmd = cmd.current_dir(resolve_run_in(ctx, ctx.args.run_in))
    return cmd.spawn(cancellation = ctx.cancellation.root).wait().code
```

### `aspect run --remote`

```python
    up = ctx.bazel.upload(t.providers.DefaultInfo.files_to_run)
    job = ctx.bazel.command(up) \
        .args(ctx.args.args) \
        .env("DOCKER_CONFIG", ".home/.docker") \
        .input(".home/.docker/config.json", ctx.std.env.home() + "/.docker/config.json") \
        .spawn(cancellation = ctx.cancellation.root)
    return job.wait().code
```

### `delivery`: many targets, spawn as built, bounded parallelism

```python
    build = ctx.bazel.build(*targets, config = cfg)
    combined = iter.combine(build, heartbeat, ctx.cancellation.root)
    for src, item in combined:
        if src == build:
            t, result = item
            if not result.success:
                rows.append((t.label, "WARN", result.failures[0].message))
            elif len(running) < max_par:
                _start(t); combined.add(running[t.label])
            else:
                queued.append(t)
        elif src == heartbeat:
            ph.update(...)
        elif src == ctx.cancellation.root:
            break
        else:
            _reap(src)
            if queued:
                t = queued.pop(0); _start(t); combined.add(running[t.label])
        if build.done and not running and not queued:
            break
```

### Watch

```python
    for src, item in iter.combine(watcher, child, ctx.cancellation.root):
        if src == watcher:
            build = ctx.bazel.build(t, config = cfg)
            result = build[t]
            build.wait()
            if not result.success or result.artifacts == last:
                continue
            last = result.artifacts
            child.cancellation.cancel(); child.wait()
            child = ctx.bazel.command(ctx.bazel.ensure(ftr)).process_group(0).spawn()
            combined.add(child)
        elif src == ctx.cancellation.root:
            break
```

Nothing is rebuilt when nothing changed: the configuration is the same value, the analysis cache is untouched, and `ensure` compares digests before touching the tree.

## Failures

- `BuildResult.failures` carries per-target outcomes as data.
- `Precondition(flag)` is raised by `ensure`, `upload` and `command` when the configuration cannot support the operation: runfile manifests disabled, no remote executor, an empty target platform. `traceback = False`.
- `RemoteError(code, operation)` is raised by `Execution.wait` on transport or server failure.
- Exit codes are values: `Child.wait().code`, `Execution.wait().code`.

## Migration

`runnable.axl` reduces to `resolve_run_in` and `RUN_IN_DESCRIPTION`. `run`, `format`, `gazelle` and `delivery` replace their `runnable(...)` tracker with `analyze`, `build`, `ensure` and `command`. The BES drain loops become `iter.combine` loops. `--build_runfile_links`, `--experimental_build_event_upload_strategy=local` and the download-floor parsing leave AXL entirely.

The local path ships first. It closes the issue on its own: `analyze`, `build`, `ensure`, `command` for an `Ensured`, and `iter.combine`. `upload`, `RemoteCommand` and `test` follow on the same records.

## The frontend

Bazel ships two things in one binary: an engine, which is Skyframe, analysis, execution and the remote module, and a frontend, which is the C++ client, the command protocol, `BlazeCommandDispatcher`, rc expansion, flags and the console. Users only ever meet the frontend, and the frontend is the problem this RFC keeps running into. It admits one command at a time. It takes every decision once per invocation, as a flag, before anything is known about the targets. It keeps state in rc files that are parsed outside the engine and that silently change what a configuration means. `--build_runfile_links` discarding the analysis cache is one symptom of that design, not a bug in it.

Buck2 did not ship this frontend. Its engine exposes analysis, build, materialization and execution to BXL scripts, and the command line is one client. The Aspect CLI today is stuck on the other side of Bazel's frontend: it can only send command lines and read BES, so everything in Appendix A is a reconstruction of state the engine already had. This RFC replaces the frontend. The four nouns are the frontend's vocabulary, AXL is its language, and the `bazel` command line is what users eject to, not what they use.

### The four categories

The engine has four seams. Each operation in this RFC sits on one, and each seam is what a class of flags configures from the outside today.

| Category | This RFC | Engine seam | Flags that cease to exist |
|---|---|---|---|
| analysis, replacing `query`, `cquery`, `aquery` | `configure`, `analyze`, `Target` | `AnalysisPhaseRunner`, `BuildView`, `BuildTool.AnalysisPostProcessor` (the hook `cquery` and `aquery` are built on), `afterTopLevelTargetAnalysis` | `--output`, `--aspects`, `--starlark:file`, `--universe_scope` |
| build | `build`, `Build` | `BuildTool.processRequest`, `BuildResultListener`, the command event bus | `--bes_*`, `--build_event_*`, `--keep_going` as anything but a per-target outcome |
| outputs: materialization, upload, download, cache | `ensure`, `upload`, `Artifact` | the output service, the remote cache and prefetcher | `--remote_download_*`, `--remote_upload_local_results`, `--build_runfile_links`, `--experimental_remote_cache_eviction_retries` |
| execution: `run`, `test`, scheduling | `command`, `test` | the spawn strategies, `ExecutionTool` | `--spawn_strategy`, `--strategy`, `--remote_executor` as a per-action choice, `--remote_default_exec_properties`, `--run_under`, `--test_strategy` |

These flags cease to exist because the decision they encode is made per target, in the task, with the target in hand. A download policy is which artifacts `ensure` was called on. A strategy is which `command` type was made. A retry is a loop. Platform selection is the dict on a `RemoteCommand`. One Starlark function executes an action locally on one branch and remotely on the other, because at this seam a spawn strategy is one interface receiving one `Spawn`.

The flags that remain are the ones that define a `Configuration`: `-c`, `--platforms`, `--define`, `--@repo//flag` and the rest of `BuildOptions`. They stay strings on `configure`, because the engine's `OptionsParser` is engine code and that vocabulary is what rules key their `select` on.

### What is removed and what stays

Removed: the C++ client and its launcher, the command gRPC protocol, `BlazeCommandDispatcher` and its server-wide command lock, `BlazeOptionHandler` and every rc file, `UiEventHandler` and the console, BES as the way a task sees its own build, and every flag in the fourth column above.

Stays: `BlazeRuntime`, `BlazeWorkspace`, `CommandEnvironment`, Skyframe, `BuildTool`, `AnalysisPhaseRunner`, `ExecutionTool`, the remote module's cache, executor and prefetcher, the rule classes and Starlark. The engine is unchanged; it is linked into a different binary.

The shape is a distribution of Bazel built from upstream sources with its own `main`. Bazel's own `main` is one line handing `BAZEL_MODULES` to `BlazeRuntime.main`; the distribution's constructs the runtime and workspace the same way and then hands control to the AXL runtime instead of the dispatcher. A task's `analyze`, `build`, `ensure` and `command` become calls on `CommandEnvironment`, `BuildTool` and the remote module. Module hooks are used where they are the right seam, `getOutputService` for `ensure` and `upload`, `registerSpawnStrategies` for `command`, not as an alternative to removing the dispatcher. The persistent server process stays, because Skyframe state is the product; the `aspect` binary on the terminal is its client over Aspect's protocol, and owns the UI.

### Runtime placement

The AXL runtime is Rust and the engine is Java. The engine runs inside the runtime: the `aspect` process creates the JVM through the JNI Invocation API with the distribution's jars on the classpath, and Java is a library in that process from then on. Rust owns the process, its threads, signals and file descriptors; Java holds Skyframe. The persistent process is the AXL engine, which the CLI already has a launcher for, and there is no server protocol because there is no second process. The runtime does not split: `std.process.Command`, `fswatch`, the pty multiplexer and the TUI stay where they are, and only the four seams cross into Java.

The boundary is handles and marshalled calls. Engine objects the runtime holds, a `CommandEnvironment`, a `BuildTool`, a configured target, are JNI global references. The four seams are calls on them, and callbacks from the engine, an event-bus subscriber, an output service, a spawn strategy, are Java classes whose methods are `native` and registered against Rust functions. A small Java bootstrap class constructs `BlazeRuntime` and `BlazeWorkspace` the way `BlazeRuntime.main` does and hands the handles back, so the Rust side never assembles the engine field by field. The four nouns were designed as exactly this boundary: `Target`, `Artifact`, `Build` and `Configuration` are the records that cross it, `analyze`, `build`, `ensure`, `upload`, `command` and `test` are the calls, and this RFC is its specification.

What this costs is what hosting a JVM always costs. HotSpot installs its own signal handlers and the host must defer to them, through `libjsig` or by installing its handlers first. Rust threads that call into the engine attach and detach. Heap sizing and the rest of today's startup options become arguments to `JNI_CreateJavaVM`, which is where the server's `--host_jvm_args` already end up. Bazel's own JNI helper library loads as it does today. None of this changes the JIT, the collector, the JDK Bazel embeds, or the set of platforms.

One consequence of hosting the JVM decides how values cross the boundary. Bazel's own Starlark runtime, `net.starlark.java`, is in the process, and every provider, `Label`, struct and attribute value a rule produced is already a `StarlarkValue` there. An AXL value can therefore be a live handle: `getattr` is `Starlark.getattr` on the Java object, a call is `Starlark.call`, `str` is `Starlark.str`, and only strings, numbers, booleans, lists and dicts are copied across. `Target.providers` in this RFC is such a handle, which is why it can carry whatever a rule exposed without a schema in Rust; `blaze_query` protos and `cquery --output=starlark` are the Appendix A approximation of it. Plain Java objects, a `Package`, a `Rule`, the runtime itself, cross the same way through reflection, which is how a task reaches an engine seam the four nouns have not named yet.

Two other placements were considered and are not the design.

- **Native Image.** GraalVM compiles the engine ahead of time into a shared library with a C ABI instead of a JVM to host. Bazel PR [#30282](https://github.com/bazelbuild/bazel/pull/30282), open since July 2026 from a BuildBuddy engineer, builds the whole server this way with Oracle GraalVM 25 and G1, carries the reachability, JNI and reflection metadata, and on a 30-revision `--nobuild` sequence over the Bazel repository with 8 GiB heaps measures 32.6 s against 41.2 s and 2.7 GiB peak RSS against 3.8 GiB. Three of its prerequisite fixes are merged. It is Linux x86-64 only, G1 under Native Image is Linux-only and Oracle-GraalVM-only, the image is about 330 MiB and takes five minutes and 11 GiB to build, and execution-heavy throughput without C2 is unmeasured. The boundary it offers is the same shape as JNI's. It is a later optimisation for memory and a single binary, taken once the invocation-API path works and only if that PR's metadata lands upstream so that it is not ours to carry.
- **The runtime inside the JVM.** [`rustc_codegen_jvm`](https://github.com/IntegralPilot/rustc_codegen_jvm) compiles Rust to JVM classes, binds Java through `#[jvm::class]` and `#[jvm::interface]`, and emits Rust traits as Java interfaces; its `foreign_interfaces` test passes Java objects into Rust and Rust objects back as implementations of Java interfaces with no glue. The runtime would be a jar with no boundary at all. It pins a nightly rustc, calls itself mid-stage, has no `std::process`, and leaves the OS-facing half of the runtime in a second process. It inverts who owns the process, which is the one thing this section decides.

### The two ways to build it

1. **A distribution against `BuildTool`.** The runtime constructs a `CommandEnvironment` through `BlazeWorkspace.initCommand` and a `BuildRequest`, and calls `processRequest` with its own `AnalysisPostProcessor` and event-bus subscribers. This is the plan. Its cost is that `BuildRequest` and `CommandEnvironment` are not public API and move between minors, so the glue is one thin layer per supported Bazel minor. That is the pin the CLI already carries, and the glue is small because `BuildTool` needs nothing from the dispatcher.
2. **A `-javaagent` into stock Bazel.** Bytecode hooks and classes generated at runtime, for a user who cannot run a different binary. Every hook is against a class not designed to be hooked, and the only points stable enough to hook are the module hooks, which do not reach the dispatcher. This is kept as a fallback for that one case and is not where the design lives.

The distribution stays a rebuild of upstream, not a fork: a different `main` and modules, no source patches. The moment it needs a patch to upstream, every Bazel release costs a rebase.

### What Appendix A becomes

| Ideal | Over the engine |
|---|---|
| `configure` | a `BuildRequest`. `flags` stay strings into the engine's `OptionsParser`. `rc` has no counterpart, because rc expansion is dispatcher code; it is the one field in this RFC that belongs to the old frontend, and it is optional for that reason. |
| `analyze` | providers read off the configured target. No `cquery`, no runtime aspect, no second invocation. |
| `Artifact.digest` | the action cache's metadata for the artifact. No BES, no execution log. |
| `ensure` | the output service's prefetcher, per artifact. No download flag and no extra invocation. |
| `upload` | the remote cache's upload of the Merkle tree. No probe action, no nonce; the target platform's exec properties come from the resolved `PlatformInfo` in process. |
| `build` streaming | an event-bus subscriber. No BES. |
| `command(up).spawn()` | the engine's own remote executor, on the channel and credentials the build used. |
| `test` | the engine's test strategy, local or remote. |
| `iter.combine` | unchanged. |

### Eject and migrate

Two things make the replacement possible to adopt. Neither weakens it.

- **Eject to vanilla.** The same four nouns run over a stock `bazel` binary, as Appendix A describes, at the cost in its third column. A task written for the frontend runs unchanged on the eject path. This is a guarantee about the API, so that choosing the frontend is never a lock-in, and it is why the API has no field that only the frontend can serve.
- **One-time migration.** A utility in the CLI translates an existing `.bazelrc` into a `configure` call once. It is finite because the flags that survive are the `BuildOptions` and the ones that do not are the fourth column, which the translation reports as decisions the task now makes. After that there is no rc. Users who want to pass Bazel flags as-is pass them to `configure(flags =)`, and the frontend rejects the ones that no longer mean anything instead of accepting them silently.

Tools that speak BES keep a BES: the event bus is engine code and emitting the protocol to a backend is a subscriber. Tools that shell out to `bazel` get the eject path.

### The catch

The thread asked what the cost is, and whether it is only ease of use. It is not ease of use. It is that the entire class of per-invocation decisions moves from flags the user sets to code the task runs, and with it the programmability the command line never had: retry, upload, execution, platform selection, all per target, all in one language. The technical cost is one thin glue layer per Bazel minor. The real cost is that it asks people to change how they use Bazel, and that is the hard part, not the engine.

## Durable state

Status: the implementation described here (`ctx.state.dump` and `hydrate`, `crates/buildx/src/state/`) is not in the tree. It is on the local branch `buildx-durable-state`, one commit on top of the engine without it. It works and is correct, but it is slower than a cold start on this repository, so it was set aside until the engine produces analysis results that are expensive to rebuild and can be serialized. The findings below stand.

The persistent process stays in the plan because Skyframe state is the product. `buildx` can also write that state down: `ctx.state.dump(path)` writes the evaluated graph of its hosted engine to disk and `ctx.state.hydrate(path)` puts it back in a later process, so the JVM may restart and the work continues. Both calls are explicit, there is no dump on exit and no background snapshot, and `crates/buildx/README.md` has the surface and the manual test.

### What is captured

The node entries of the loading graph with their values, versions and edges: loaded packages, globs, transitive-target values, file and directory state, package lookups, written through Bazel's own `ObjectCodecs`. After the three queries of `state-dump.star` on the repository root that is 56,907 of 75,433 nodes in 7.9 MB. The other 18,525, the `.bzl` modules, module resolution and what they depend on, hold code Bazel's codecs cannot write (a `Module` of Starlark functions; a Java `record` in the module-resolution values) or are what those need, so a hydrating engine evaluates them again and attaches the restored nodes to them, comparing each by a digest. The restored values name the `.bzl` globals they refer to rather than their position, so a hydrating engine resolves each name against the modules it just loaded: adding or removing a global, or renaming a `.bzl`, no longer invalidates anything but the nodes that load the changed module. A hydrate validates the Bazel version, the workspace path, a format version and the body's length and digest before an engine starts; what changed on disk since is invalidated by Skyframe's ordinary file diffing at the next command, and a changed `.bzl` drops the restored nodes that depend on it.

### What is not captured

Repository rule outputs on disk (they are the output base's and are reused), in-flight actions, the script's Starlark values, live JNI handles, and analysis: the engine does not run analysis yet, so there are no configured-target or action nodes to keep. When it does, Bazel's frontier serializer is the mechanism meant for those nodes.

### Version coupling and platforms

A dump is valid for the exact Bazel version, buildx bindings, workspace path and engine configuration that wrote it, because the values are written by reflection over Bazel's own classes. It is written and verified on macOS arm64. Linux x86_64 and arm64 are expected to behave the same, since nothing in the mechanism is platform specific, and are not tested.

### Mechanisms measured

All four were tried on Bazel 9.0.1 over the repository root (399 targets, 529 packages; 73,696 graph nodes after `deps(//:all)`, 75,429 after the three queries of `state-dump.star`).

| Mechanism | macOS arm64 | Linux | Result |
|---|---|---|---|
| Bazel's frontier serializer, `--experimental_remote_analysis_cache*` | runs, captures nothing | same | It selects only analysis and execution nodes, of which a loading graph has none (none of its 73,696 nodes). `PackageCodec` needs a `PackageSerializer` the open-source jar does not ship, `RuleCodec` refuses by design, and the only store is in memory. Its command module is not in the jar. Not usable. |
| CRaC (Azul Zulu 25 CRaC) | the JDK exists, only the `simengine` is in it: a checkpoint wrote 842 bytes and the same process went on | Zulu 25 CRaC JDKs exist for x64 and arm64, glibc and musl; paper only | Cannot restore on macOS (measured). On Linux it needs the `criu` engine, privileges, every open file descriptor closed (Bazel holds jars, the output base lock and sockets), and a `libjvm` hosted by a launcher, where buildx hosts it through JNI from a Rust process. Not tried: no Linux environment. |
| CRIU on the whole `buildx` process | none | Linux only; paper only | Image of the whole resident set, privileges, same kernel and paths on restore; it would restore at memory speed and capture everything, JVM and handles included. Not tried: no Linux environment. |
| Custom dump over `ObjectCodecs` (this section) | works | expected; untested | Dump 2.3 s and 7.9 MB, hydrate 1.6 s; 399 / 23310 / 736 unchanged; a BUILD edit, an edited, extended, renamed or global-removing `.bzl`, a changed `MODULE.bazel`, an added source and a deleted BUILD file all match a cold run. |

Docker's CLI is installed on the development machine but its daemon was not running, and there is no Colima, Lima or OrbStack, so the Linux rows are analysis from documentation, not measurements.

The custom dump is the only one that restores on macOS, and on this repository it is still not faster than a cold start. A cold `deps(//:all)` is 1.98 s as a whole process; a hydrated one is 2.40 s, of which the hydrate is 1.61 s: 1.0 s evaluating the nodes it cannot restore and 0.5 s restoring. Restoring on four or more threads, each attached to the JVM and inserting nodes through the typed bindings, took the restore from 0.95 s to 0.50 s (2 threads 0.68 s) and the whole run from 2.88 s to 2.40 s; it stops there because reading the graph is Bazel's single-threaded decoder (0.14 s) and the rest is about 17 JNI calls per node. A synthetic workspace of 3,000 light packages measured 2.1 s cold and 2.2 s hydrated, and one of 1,000 packages of 240 targets 5.7 s and 5.5 s (before the parallel restore). It pays for itself only where loading is slower than restoring: slow or cold disks, few cores, heavy BUILD evaluation. A process-level checkpoint is the mechanism that restores at memory speed, and it is Linux only.

The 1.0 s is the floor of this design. The 1,028 `.bzl` loads cannot be written: Bazel's own codecs write a `Module` only as a reference (load key and global name), resolved by a Skyframe lookup that evaluates the file again, and the code itself (functions, closures, the compiled program) has no codec. The module-resolution values hold a `record`, which `DynamicCodec` cannot write; they must not be trusted in any case. Everything else in the 18,525 is a dependency of those, and every one is needed to attach the restored nodes, so none can be deferred to the first query. A thread-dump profile of that evaluation attributes about 40% of the samples to `.bzl` loading, 25% to the file and directory nodes under it, 10% to `bzlmod` and the rest to repository nodes.

Decision: macOS is a first-class platform, so the Linux-only checkpoint routes (CRaC, including its Warp engine, and CRIU) are not pursued. Durable state is the Skyframe dump, which has to earn its keep on macOS. Done: the restore runs on all cores, and the values refer to `.bzl` globals by name, so a hydrate keeps everything the edits since the dump did not reach. Not done, and not possible from here: restoring the `.bzl` and module nodes themselves, which needs Java-side codecs (a `Module`'s code, a `record`), and putting entries into the graph's node map in one call, which needs a blazon binding for `java.util.concurrent.ConcurrentHashMap`. Neither is likely to matter until analysis results are in the graph.

## Alternatives considered

Four independent specifications were written against the same requirements from skeptical, minimalist, maximalist and layered stances. All four converged on no `run` verb, a per-target completion stream, Bazel-named records and `std.process.Command` as the local surface. They differed on a mode switch on `build`, which this RFC rejects in favour of explicit materialization, and on whether remote execution is a separate `action` record, which this RFC replaces with a second `command` type. Seven call-site sketches of candidate shapes preceded them; the BXL-style handle won.

Rejected along the way, in order: a `ctx.bazel.run` verb mirroring `build`'s parameters; a knob per requirement (`mounts =`, `home =`, `remote =`); a fully decomposed REAPI stack in AXL (`Tree`, `remote.Command`, CAS reads); a `RunSpec` record shared by local and remote; `run = "local" | "remote"` on `build`; `ctx.bazel.action` plus `execute`; a `select` over waitables, which needs `while`.

Buck2's BXL is the closest prior art. Taken: per-target results with per-target failures, artifacts as values, materialization as its own step, analysis as a first-class call. Rejected: synchronous build with results at the end, deferred end-of-script `ensure`, and running inside the action graph.

## Open questions

1. Should `ensure` under a `minimal` download mode run the follow-up download itself, or should that be a `Precondition` that tells the caller to configure `toplevel`?
2. Should `Runfiles.files` be eagerly populated on `Target`, or only when `upload` or a caller reads it? A JS tree is tens of thousands of entries per target.
3. Does `analyze` on a build that has already run reuse that build's providers, or always cost its own invocation?
4. `upload` or `stage` as the name for the CAS destination.
5. Hosting the JVM: one `CommandEnvironment` per task or per `build` call, how long a `Build` handle may hold its environment open while the next task starts, and whether the engine's command lock is replaced by Skyframe's own serialization or by one in the runtime.

## Appendix A: over the `bazel` binary

The eject path. The same API, served by sending command lines to a stock `bazel` and reconstructing from BES, `cquery` and the execution log what the engine already knew.

| Ideal | Over the `bazel` binary | Where the cost lands |
|---|---|---|
| `configure` | rc handle plus flags | a record the runtime expands per invocation |
| `analyze` | `cquery --output=starlark` for providers, the runtime's aspect for expanded `args`, the target platform label and `RunEnvironmentInfo` | one invocation, or none when the build already carried the aspect |
| `Artifact.digest` | BES `File.digest` for top-level outputs, the compact execution log for runfiles | inside `upload` |
| `ensure` | manifest-driven tree reconcile; under `minimal`, one incremental invocation with `--remote_download_outputs=toplevel`, a command option with no cache cost | inside `ensure` |
| `upload` | an always-executing probe action over `files_to_run`, nonce-keyed, so Bazel uploads the tree and the execution log reports digests; target platform resolved from the label through `parents` | one trivial remote action per target |
| `build` streaming | BES `target_completed` joined with its filesets in Rust | none |
| `command(up).spawn()` | the generated REAPI clients in `axl-proto` | `FindMissingBlobs` plus host-file uploads |
| `test` | `bazel test` remotely; locally the runtime sets `TEST_*` | none |
| `iter.combine` | a new runtime primitive over the existing async handles | none |

Two facts about Bazel decide the remote mapping. An aspect's action runs on the aspect's exec platform, not the target's, so the real execution must take its platform from `Target.platform`, which the aspect records as a label and the runtime resolves. And `PlatformInfo` does not expose `exec_properties` to Starlark, so that resolution is a query, not a provider read.
