# buildx

Proof of concept for the engine path in `docs/rfc/buildx`: Bazel's engine as a
library inside this process, driven by a Starlark script shaped after Buck2's
BXL, without Bazel's command layer.

```
buildx path/to/script.star:main
```

```python
def _impl(ctx: Context):
    for t in ctx.analysis().query("//..."):
        ctx.output.print(t.target_kind, t.label)

main = buildx_main(impl = _impl, cli_args = {})
```

The script's workspace is the nearest ancestor of the current directory with a
`MODULE.bazel` or `WORKSPACE`, and its `.bazelversion` picks the Bazel release,
downloaded on first use into the user cache
(`~/Library/Caches/buildx/bazel/<version>` on macOS). The engine starts on the
first `ctx.analysis()` and lives for the rest of the script. `impl` may return
an int, which becomes the exit code. `cli_args` must be `{}` for now.

## How it works

`buildx` creates a JVM through the JNI Invocation API from the JDK that ships
inside the release, puts that release's `A-server.jar` on the classpath, and
constructs `BlazeRuntime` the way `BlazeRuntime.main` would. There is no client,
no gRPC server, no console, no rc file and no command: no `CommandEnvironment`,
`BlazeCommand` or `BlazeCommandDispatcher` is created. Package loading and
queries are driven on `BlazeRuntime` -> `BlazeWorkspace` -> `SkyframeExecutor`
directly, and results stay on the heap. Skyframe stays warm between calls
because the process stays up.

The calls a query takes, per query: each module's `getPrecomputedValues()`
injected with `SkyframeExecutor.injectExtraPrecomputedValues`, then
`SkyframeExecutor.sync(...)`, then a `QueryEnvironment` from
`runtime.getQueryEnvironmentFactory().create(...)`, `QueryExpression.parse`,
`evaluateQuery` into `QueryUtil.newOrderedAggregateAllOutputFormatterCallback`,
`close`, and `notifyCommandComplete`. Once, after `newRuntime`: a
`PathPackageLocator`, an `OptionsParser` over the option classes loading reads
(parsed with no arguments), `TimestampGranularityMonitor`,
`QuiescingExecutorsImpl.forTesting()` and a `Reporter`.

`BazelRepositoryModule.beforeCommand` needs a `CommandEnvironment`, which
buildx never creates, so `seed_repository_module` builds the same values from
the option defaults (an `OptionsParser` over the common option classes, parsed
with no arguments) and the process environment, through the generated typed
members of the module's private fields:

- the client env (the process environment) and the repo env (`PATH` only, as
  `--incompatible_strict_repo_env` defaults), as the module's two suppliers and
  as `sync`'s client env;
- `clock`, default `registries` and `moduleMirrors`;
- a `RepositoryCache` at `<output user root>/cache/repos/v1` with hardlink
  from `--experimental_repository_cache_hardlinks`, and the repo contents
  cache's shared lock;
- a `DownloadManager` over an `HttpDownloader` and `DelegatingDownloader`
  from `--http_connector_attempts`, `--http_connector_retry_max_timeout`,
  `--http_max_parallel_downloads`, `--http_timeout_scaling`,
  `--repository_downloader_retries` and `--experimental_scale_timeouts`, given
  to the repository fetch, module file, repo spec, yanked versions,
  single-extension and vendor functions;
- a `ProcessWrapper` over the release's `process-wrapper` (no kill delay, no
  graceful SIGTERM, as for a command without local execution options) and
  `SyscallCache.NO_CACHE`;
- `BazelLockFileModule`'s executor, workspace, output base and lockfile mode,
  so its `afterCommand` writes `MODULE.bazel.lock` after each command, as a
  `bazel query` does.

Each command also injects every module's `getPrecomputedValues()`, then
`sync`s. Not seeded, so only their defaults apply: credential helpers and
`.netrc`, the downloader config URL rewriter, `--distdir`, remote repository
execution and the remote repo contents cache, repo contents cache garbage
collection, and every flag a `.bazelrc` would set (there are no rc files).
Query results match `bazel query` on this repository for `//...` (399),
`deps(//:all)` (23310), `@crates//:all` (736) and `buildfiles(...)` (111).

Bazel classes are called through typed handles generated into `OUT_DIR` by
`build.rs` from `bindings.toml`, one module per version in its `versions`,
via `blazon`; the runtime pieces live in `blazon-rt`. See `crates/blazon`.
Passing the wrong handle to a Bazel method does not compile. The engine is
bound to one version at a time and refuses another with a message naming the
regeneration command.

## Script surface

- `ctx.analysis().query(expr)` returns the live result set of the query (a
  `java.util.Set` view) whose elements are generated `Rule` and `InputFile`
  values. `Engine::query` is the Rust entry point.
- `ctx.analysis().package("//lib")` returns the live `Package` object, loaded on demand.
- `ctx.java.runtime` is the live `BlazeRuntime` as its generated Starlark type.
- `ctx.output.print(*args)` writes to stdout; `print` goes to stderr.
- `ctx.cli_args` is an empty struct.
- `ctx.config(**settings)`, `ctx.analysis().analyze(targets, config = cfg)`,
  `ctx.build(targets, config = cfg, executor = e)`, `ctx.outputs.ensure(...)`
  and `ctx.execution.local()`: described under "Configuration, analysis and
  build" below.

Starlark values for Bazel's classes are generated, not reflected: with
`starlark = true` in `bindings.toml`, `blazon` emits a Starlark type per bound
class (`Rule`, `Label`, `Package`, `BlazeRuntime`, ...), registered once by
`register()`, and `type(x)` is the generated name. Classes without
`@StarlarkBuiltin` get their members from the spec: `@getRuleClass` is the
attribute `rule_class`, `getAttr(Ljava/lang/String;) as attr` the method `attr`. A
script never sees a Java-style name: a spec member is exposed under its Rust name
(`getRuleClass` is `rule_class`, `setClock` is `set_clock`) and ` as name` renames
it; members Bazel declares with `@StarlarkMethod` keep Bazel's names.
Query results and packages reach the script through `blazon_starlark::wrap`,
which picks the most specific generated type, so a `Set<Target>` iterates as
`Rule` and `InputFile` values.

Spec members resolve through the class, its superclasses and interfaces, so
`label`, `name`, `attr` and `visibility` declared on `RuleOrMacroInstance`
reach `Rule`, and `Packageoid.targets` reaches `Package`. A `Target` kind that
no spec entry names (`OutputFile`, `PackageGroup`, `InputFile`) shows up as
the nearest bound ancestor, so list `@getTargetKind` for each kind a script
may see.

## Configuration, analysis and build

```python
cfg = ctx.config(compilation_mode = "opt", copt = ["-O3"], action_env = {"A": "b"})
analyzed = ctx.analysis().analyze(["//:app", "//lib:all"], config = cfg)
for label, t in analyzed.items():      # dict[Label, target or Failure]
    if type(t) == "Failure":
        print(label, t.message)
        continue
    info = t[ctx.analysis().providers.DefaultInfo]
    print(label, [f.short_path for f in info.files.to_list()])

results = ctx.build(["//:app"], config = cfg, executor = ctx.execution.local())
for label, r in results.items():       # dict[Label, BuildResult]
    for f in r.failures():             # data, not exceptions
        print(label, f.message)
    for a in r.artifacts():            # path, digest, size, is_local are free
        print(a.path, a.digest, a.size, a.is_local)
print(ctx.outputs.ensure(results))     # verifies files exist; trees expand
```

`ctx.config(**settings)` takes keyword settings named like Bazel's options
without dashes: bools are `True`/`False`, list options are lists, assignment
options (`action_env`) are dicts, labels and paths are strings, enums are
strings checked against the option's values, and `starlark = {"//pkg:flag": v}`
sets Starlark build settings. Every name and type is checked against the option
schema that `blazon` generates from the release (unknown names suggest close
matches, wrong types name the setting and the expected kind), converted to the
option string form internally and parsed by the engine's own `OptionsParser`,
so Bazel's converters and validation apply; option strings never appear in the
API or in messages. `ctx.config().describe("compilation_mode")` returns the
setting's `kind`, `default`, `help` and `effect_tags`. `==` and hashing hold for
equal settings regardless of their order. A deprecated setting warns.

Settings of a configuration fragment form the `BuildOptions` half of a `Config`.
The rest (Starlark semantics, package loading, repository and bzlmod settings)
are the engine half, and take effect from the `Config` used for a call:

- Starlark semantics (`BuildLanguageOptions`), `deleted_packages`,
  `default_visibility`, `--action_env` and the platform mapping are read by
  `SkyframeExecutor.sync` from the `Config`'s parser, as for a command.
- Before a call whose engine half differs from the last applied one, buildx
  does what `BazelRepositoryModule.beforeCommand`, `BazelLockFileModule.beforeCommand`
  and the dispatcher do from a command's options (`engine/apply.rs`):
  `package_path` rebuilds the `PathPackageLocator`; the repository module's
  fields are rewritten (`override_repository`, `inject_repository`,
  `override_module`, `registry`, `module_mirrors`, `check_direct_dependencies`,
  `bazel_compatibility_mode`, `lockfile_mode`, `allowed_yanked_versions`,
  `vendor_dir`, `fetch`, `repository_cache`); a new `DownloadManager` takes
  `disable_download`, the downloader retries, `distdir`, the hardlink and timeout
  scaling settings; `repo_env` sets the repository environment; the lockfile
  module takes `lockfile_mode`. Each `sync` then injects the module's precomputed
  values, so Skyframe invalidates what depends on a changed one (module
  resolution, `.bzl` loading, repository fetches) exactly as for a command that
  changed the flag.
- `ignore_dev_dependency` lives in an `AtomicBoolean` the bindings cannot
  express, so it is injected as a precomputed value after the module's own.
- Equal engine halves cost nothing: a fingerprint of the engine half's settings
  is compared with the one last applied, and a match skips everything above.
  Only one engine half is live at a time, so a script that alternates two
  `Config`s with different engine halves pays the re-evaluation (module
  resolution, loading) each time; a `Config` with only fragment settings has
  the default engine half. Default calls (`query(expr)` without `config`) apply
  the default half.

Not applied: the settings seeded once at start that no Bazel command changes
mid-process (the process wrapper, the repo contents cache location and its
lock, credential helpers, `.netrc`, the downloader config, remote repository
execution and the repo contents cache GC), `repository_downloader_retries` back
to its unset state, and the vendor directory's symlinks (`vendor_dir` sets the
vendor directory for resolution only). Builds need `--keep_going` set internally
and ignore remote and sandbox settings, which are not config. There are no rc
files.

`repo_env` reaches the repository rules' environment (the `PATH`-only strict
form or the client environment, with the `Config`'s assignments on top), and
changing it re-fetches the repositories that read it, in the same process
(`examples/repo-env.star`). Targets given as strings are resolved with the
`Config` synced too, and `ctx.analysis().query(expr, config = cfg)` takes one.

`analyze` runs what `BuildView.update` does for a build that stops after
analysis, without a command: `SkyframeExecutor.sync` over the `Config`'s
parser, `setBaselineConfiguration`, `createConfiguration`, the
`SkyframeBuildView`'s `shouldDiscardAnalysisCache`, `setConfiguration` and
`configureTargets` over `ConfiguredTargetKey`s, with a `TopLevelArtifactContext`
built the way `BuildRequest` builds it. Targets are label or pattern strings
(run as queries), live targets, or whole query results. The result is the live
configured targets, as a `dict[Label, ...]` whose values are generated `Target` values with `label`, `files`,
`files_to_run`, `output_groups` and indexing by provider; the provider keys
`DefaultInfo`, `OutputGroupInfo` and `RunEnvironmentInfo` are on
`ctx.analysis().providers`. A target whose analysis fails (bad attribute,
missing or invisible dependency, `fail()`) is data: its label maps to a
`Failure` with `label` and `message` (Bazel's own error text for that target),
and the others still analyze, since `configureTargets` runs with `keep_going`.
The same targets in `ctx.build` get a `BuildResult` with that failure in
`failures()` and `target` None, while the rest build. Only what cannot be
attributed to a target (the engine, a bad setting, an unparsable pattern)
raises.

`build` blocks. After the analysis phase it does what `ExecutionTool` and
`SkyframeBuilder` do, assembled by hand: artifact roots and the source forest
from the analysis' package roots, a `LocalOutputService`, the action log
directory, the persistent action cache with an `ActionCacheChecker`, a
`SingleBuildFileCache`, a `ResourceManager`, a `BlazeExecutor` over a
`ModuleActionContextRegistry` (spawn strategy resolver, file write, template
expansion, symlink tree) and a `SpawnStrategyRegistry` whose standalone strategy
runs a `LocalSpawnRunner` over the release's `process-wrapper`, then
`detectModifiedOutputFiles`, `configureActionExecutor` and
`SkyframeExecutor.buildArtifacts` on the targets' completion keys. A build
always attempts every target (`keep_going` is set on the parser, not exposed),
so a failing target is a `Failure` in its `BuildResult` while the others build.
Artifact metadata comes from Skyframe's `ActionExecutionValue`,
`TreeArtifactValue` and `FileArtifactValue`; nothing is downloaded or uploaded.
`ctx.outputs.ensure` accepts built artifacts, results or lists of them and, for
the local executor, checks each file exists. `ctx.build`'s `executor` may only
be `ctx.execution.local()` (or omitted); anything else is a "not implemented"
error.

Not implemented: other executors, stores, caches and combinators; remote
execution and remote metadata (`is_local` is always true in practice); tests
(`TestActionContext`) and coverage; aspects; convenience symlinks; build event streams, profiles and progress
output; cancellation and watching.

### Workspace status and stamping

Every build runs Bazel's own workspace status action
(`BazelWorkspaceStatusModule$BazelWorkspaceStatusAction`, created by the
`SkyframeExecutor`'s `WorkspaceStatusAction.Factory`, which the module's
`workspaceInit` registers): `getWorkspaceStatusArtifacts` evaluates it and its
two artifacts, `stable-status.txt` and `volatile-status.txt`, go into the
`artifactsToBuild` of `buildArtifacts`, as `BuildView.update` does for a
command. The action is `executeUnconditionally`, so it runs on every build, and
a target that reads a status file (`stamp = 1`, `--stamp` for rules that
honour it, `ctx.info_file`, `ctx.version_file`) depends on it as an input.

```python
cfg = ctx.config(
    stamp = True,
    workspace_status_command = "tools/status.sh",
    embed_label = "release-1",
)
ctx.build(["//:release"], config = cfg)
```

- `workspace_status_command` is a command run by `sh -c` with the workspace as
  its directory and the process environment, exactly as `CommandBuilder` in
  `BazelWorkspaceStatusModule` does. Its path is a `PathFragment`, so it is
  normalized as for the flag: `./status.sh` becomes `status.sh`, which the shell
  looks up in `PATH`; spell a script in the workspace root `tools/x.sh` or give
  an absolute path. Its output is `KEY value` lines; keys starting with
  `STABLE_` are stable, the rest volatile.
- `embed_label` is `BUILD_EMBED_LABEL` in the stable file. `stamp` is the
  configuration's own option.
- Stable status holds `BUILD_EMBED_LABEL`, `BUILD_HOST`, `BUILD_USER` and the
  command's `STABLE_*` keys; volatile status holds `BUILD_TIMESTAMP`,
  `FORMATTED_DATE` (`SOURCE_DATE_EPOCH` is honoured) and the command's other
  keys. The stable file is rewritten only when its content changed, and the
  volatile one is constant metadata: a changed stable value rebuilds the
  stamped targets, a changed volatile one rebuilds nothing, and unstamped
  targets never depend on either. A changed `workspace_status_command` takes
  effect on the next build in the same process, since the action reads it from
  that build's configuration.
- The `workspace_status` event in the stream is the `BuildInfoEvent` the action
  posts, so it carries every key. If the action did not run (a build that
  analyzed nothing) the event buildx posts instead holds `BUILD_HOST`,
  `BUILD_USER` and `BUILD_TIMESTAMP`; if it failed, the event is empty, as
  Bazel's.
- A failing command fails the build: `ctx.build` raises with Bazel's message
  (`Failed to determine workspace status: Process exited with status N`),
  because the failure belongs to no target. The command's stderr is not
  repeated in the message.

What buildx constructs without a `CommandEnvironment`: the module's
`WorkspaceStatusAction.Context` (private, over a `CommandEnvironment`) is
implemented in Rust in `engine/status.rs` and registered in the build's action
context registry, restricted as `ExecutionTool` does: `getOptions` is the
`WorkspaceStatusAction.Options` class of the build's parser (added to the
option classes buildx parses), `getClientEnv` the process environment, and
`getCommand` a `CommandBuilder` over that environment, in the workspace, with
the shell. Everything else is Bazel's.

Stamping and caching. A stamped action reads `stable-status.txt` as an input, so
its action key changes whenever a stable value (a commit, `BUILD_USER`,
`BUILD_HOST`, `embed_label`) changes. With `ctx.execution.cached(store, ...)`
the cache key is the REAPI `Action` digest, which contains the input root, so a
stamped action gets a different key for every stable status: a cache hit
needs the same stable status, and a build stamped with a fresh commit misses on
every stamped action and on everything downstream. Unstamped actions are
unaffected, and so are builds that change only volatile keys. The persistent
action cache behaves the same way.

C++ builds: `CppCompileAction` needs a `CppIncludeExtractionContext` and
`CppIncludeScanningContext` in the registry, which `engine/contexts.rs`
implements in Rust (the build's `ArtifactFactory` as resolver; no additional
inputs, as with include scanning off). The rules_cc auto-configured toolchain on
macOS needs `features = ["no_dotd_file"]` in the config here, as it does for
Bazel itself on this machine (the dependency file names an SDK file outside
the toolchain's include directories).
A build failure's message is the `ErrorInfo` exception's message and its causes.

## No-op builds, and `BUILDX_PHASES`

`BUILDX_PHASES=1` makes every `ctx.build` print a wall-clock table of its phases
to stderr: engine start, the sync, target resolution, analysis (configuration,
`configureTargets`), the execution setup, `buildArtifacts`, the results and
the Starlark values. Off, a phase costs one atomic load. It is the tool for
"why is this build slow".

What a repeated build skips or shares:

- Target patterns are evaluated inside the build's own synced graph: one sync
  per call instead of one for the query and one for the build, and the query's
  result set stays a Java collection instead of becoming 10,000 Starlark
  values and back.
- The configured target keys and the labels of a target set are reused while
  the targets (compared with `Set.equals`: unchanged packages give the same
  objects) and the top-level configuration are equal. A BUILD file edit gives
  different targets, so the keys are rebuilt.
- `BuildResult.artifacts()`, `.target` and the artifacts' metadata are read when
  asked for, from the batch of the build. A script that only counts results,
  or only reads a few, does no per-target work. Results are read from the
  build's graph at that time, so an earlier build's result asked after a later
  build that rebuilt the artifact shows the newer one. Failures are looked up
  per target only when the evaluation reported an error.
- Everything else per build is Bazel's own and small: the sync diff, the
  configuration, `configureTargets` over unchanged keys, `detectModifiedOutputFiles`
  (a stat per output, as Bazel does) and `buildArtifacts` checking each action.

Measured on `examples/large_bes` (10,102 targets, a scratch copy), release
build; a debug build is about 3 to 8 times slower in the Rust marshalling:

| Warm no-op `ctx.build(["//..."])` | before | after |
|---|---|---|
| release | 440 ms | 130 ms |
| debug | 1170 ms | 145 ms |

A fresh process's no-op build takes 3.4 to 3.8 s: engine start 0.6 s, loading
the packages (target resolution) 0.6 s, analysis 1.4 s (`configureTargets` over
10,000 targets, which Bazel does in the same time), and `buildArtifacts` 0.5 s
(checking 10,000 actions against the action cache).

Action keys. Bazel's persistent action cache entries include the action
execution salt that `RemoteModule` hands the executor builder: a digest of
"remote cache enabled" and the default remote exec properties, which is a
non-empty string even with no remote options. buildx used to pass an empty salt,
so a plain `bazel build` after buildx saw every action as "changed since cached
execution" and re-ran it (10,502 actions on `large_bes`), and the other way
round. buildx now computes the same digest (`false` and no properties), and the
two share action cache entries in both directions. With `--remote_cache` flags
Bazel's salt differs; buildx has no remote options, so it does not.

## Build progress

While `ctx.build` runs, buildx writes a progress line to stderr, always on,
with no setting: `buildx: [5733/10101] actions, 12s`, at most once a second, and
a last line when the build ends (`..., 14s, 1 failed` when an action failed).
Builds that finish within about a second print nothing, so a repeated build with
nothing to do is silent. On a terminal the line is rewritten in place with a
carriage return; otherwise each line is printed on its own. Stdout is never
touched, so `ctx.output.print` and the progress line do not interleave.

The counts are Bazel's own `ExecutionProgressReceiver`, handed to
`SkyframeExecutor.buildArtifacts`: it counts every action that completed,
actions served from a cache (local, remote or `cached`) and failed actions
included, over the actions enqueued so far. The total is not known up front: it
grows while analysis discovers actions, so the second number is the total so far,
as in Bazel's own `[n / m]`. A thread reads the counter once a second; nothing
runs per action, and the per-action cost on 300 genrules is unchanged (about 2.9 ms
with and without it). The `failed` count is the number of errors Bazel reported
while the build ran (one per failed action), kept by a handler attached to the
build's reporter, so it also counts other errors such as a failed `Execute`
transport.

## Execution, routing, remote and cache

Execution has three parts: an `Executor` runs a spawn, a `Router` picks an
executor per spawn, and a Rust `SpawnStrategy` sits in front of Bazel's own
local one, registered first in the `SpawnStrategyRegistry` (`engine/strategy.rs`).
For every spawn Bazel's `SpawnStrategyResolver` asks it `canExec(spawn, context)`;
it reads the spawn, asks the router, and answers false when the chosen executor
is Bazel's own local strategy, so the next registered strategy,
`StandaloneSpawnStrategy` over the `LocalSpawnRunner`, executes the spawn as it
always did. Every other executor answers true, and the Rust `exec` runs it.

Can a non-local executor run inside the Rust `exec` and report success and
failure correctly? Yes. A failed spawn is thrown as Bazel's own
`SpawnExecException` holding a failed `SpawnResult` (status `NON_ZERO_EXIT` and
the exit code for a command that exited non-zero, `EXECUTION_FAILED` for
transport errors, routing errors and the like, with a `FailureDetail` either
way), the same thing `AbstractSpawnStrategy` throws, so the action fails with
`(Exit N): ...` and `BuildResult.failures()` shows it while the rest of the
build continues. `blazon_rt::Error` keeps the Java `Throwable`, so an exception
from a Java delegate call, Bazel's own strategy running under `cached`,
propagates through `?` unchanged. Success is a `SpawnResult` with the runner
name and whether it was a cache hit; stdout and stderr are written to the
action's `FileOutErr`. The plain local executor still answers false so that
the common path stays entirely Bazel's.

The REAPI view of a spawn (`reapi.rs`, `execution.rs`), built lazily when a
router or executor asks (a default build never builds it):

- `Command`: arguments, environment variables sorted by name, `output_paths`
  sorted, platform properties (the spawn's merged exec properties) sorted,
  working directory.
- The input root as a `Directory` tree (`Tree`: root and every child directory)
  with SHA-256 digests, from `SpawnInputExpander.getInputMapping`, Bazel's own
  input mapping: tree artifacts expanded to their files (empty ones kept as
  directories), runfiles in their `.runfiles` layout, unresolved symlink
  artifacts as `SymlinkNode`s, parameter files and other in-memory inputs with
  their bytes. File digests and sizes are the ones Skyframe recorded in
  `FileArtifactValue` (a file is hashed from disk when Bazel recorded none) and
  the executable bit comes from the file. Filesets appear as their links.
  Nothing is cached per action key.
- `Action`: the command and input root digests, the `timeout` tag,
  `do_not_cache` for the `no-cache`, `no-remote-cache` and `local` tags, and the
  platform.

`Store` (`store.rs`) is the CAS and action cache of one endpoint and instance
over the `axl-proto` clients: `FindMissingBlobs`, `BatchUpdateBlobs` and
`BatchReadBlobs` for blobs under the server's batch limit (from `Capabilities`),
ByteStream `Write` and `Read` above it, `GetActionResult` and
`UpdateActionResult`. Compression is off, a channel is made lazily per store,
and the headers given to `ctx.execution.store` are sent on every call and never
logged. `RemoteExecutor` (`remote.rs`) uploads what the server lacks (the
`Command`, `Action`, every `Directory` and the input files, read from the
execution root or from the in-memory input), calls `Execute`, follows the
operation to its end, and downloads every output file and directory of the
`ActionResult` into the execution root with parents created and executable bits
set; stdout and stderr go to the action's streams, and a non-zero exit code is
the failure. `Cached` looks the action up by its REAPI `Action` digest: a hit
downloads the outputs and the action does not run; a miss runs the inner
executor and, when it succeeded, uploads the outputs read from the execution
root and stores an `ActionResult` (a result the inner executor already has, a
remote one, is stored as it is); a failed result is never stored. An unusable
cache entry is a miss. Because the cache runs the inner executor itself,
`cached(store, local)` runs Bazel's local strategy as a delegate from the Rust
`exec`, with its result standing and its exception propagating.

Scripts: `ctx.execution.local(name = ...)`, `ctx.execution.store(endpoint,
instance = "", headers = {})`, `ctx.execution.remote(endpoint, instance = "",
store = None, name = None)`, `ctx.execution.cached(store, executor)` and
`ctx.execution.route(fn, executors = {...})` return executor values (and a
store value); `ctx.build(executor = ...)` takes one. `route` returns an
executor that calls `fn(spawn)` for every spawn and expects one of `executors`
back (also `spawn.executors`, so the top-level function can name them); a plain
function passed as `ctx.build(executor = fn)` routes to `ctx.execution.local()`.
`Spawn` has `mnemonic`, `execution_info`, `platform_properties`, `arguments`,
`environment`, `working_directory`, `outputs`, `executors`, `action_digest` and
the protos `action`, `command` and `input_tree` as `axl-proto` messages.
Bazel calls the function on its own threads, one evaluator per call, so it must
be frozen: the script's module is frozen before `impl` runs, and a closure or
lambda created while it runs is refused with an error saying so. A function that
raises fails that action with its message.

`examples/fake_remote.rs` (`cargo run -p buildx --example fake_remote -- --port
17890`) is an in-memory REAPI server for the remote and cache examples, a test
fixture and not a product: CAS, ByteStream, action cache, capabilities and
`Execute`, which materializes the input root, runs `arguments` directly with the
given environment and collects the declared outputs. It logs each `Execute`
and serves its counters as JSON over HTTP on the port it prints. There is no
authentication or TLS in it, and no execution priorities, retries or
`WaitExecution`.

## Build events: BEP and BES

`ctx.build(targets, events = [sink, ...])` streams the build's Build Event
Protocol, per build like the executor. A script that passes no `events` pays
nothing. Every `ctx.build` with sinks is one invocation: one invocation id (a
UUID), one build id, one stream. Two sinks of one build get the same events.

```python
ctx.build(
    ["//..."],
    events = [
        ctx.events.file("build.bep"),
        ctx.events.bes("grpcs://bes.example:443",
                       headers = {"x-api-key": "..."}, instance = "main",
                       results_url = "https://results.example/invocation/",
                       metadata = {"ci": "false"}),
    ],
)
```

- `ctx.events.file(path)`: delimited `BuildEvent` messages, what
  `--build_event_binary_file` writes. The file is truncated per build.
- `ctx.events.bes(endpoint, headers, instance, results_url, metadata,
  artifacts)`: a Build Event Service at `grpc://host:port` or
  `grpcs://host:port`. `headers` are sent with every call and never logged or
  printed; names are letters, digits, `-`, `_` and `.`, sent in lowercase, so
  `X-Aspect` works and two spellings of one name are an error. `instance` is the BES instance
  (project) name. `metadata` is what `--build_metadata` does: it becomes the
  `build_metadata` event. `results_url` is a base URL: once the stream was
  received in full, `buildx: results: <results_url><invocation id>` goes to
  stderr (it is not printed when the upload failed).
- Arguments are checked when the sink is made: an empty or schemeless endpoint,
  an empty or unwritable file path, a bad header name or value, an empty
  metadata key are errors naming the argument.

The transports are Bazel's own. A `BuildEventStreamer` is registered on the
event bus the engine already builds (the one `Reporter` and the progress counter
use), so everything Skyframe posts while the build runs (target configured and
completed, action failures, named sets of files, configurations) reaches the
sinks through Bazel's code, in Bazel's shapes. A file sink is
`BinaryFormatFileTransport`; a BES sink is `BuildEventServiceTransport` over
`BuildEventServiceGrpcClient` (retries, acks, ordering and half close are
Bazel's), on a Netty `ManagedChannel` built with `NettyChannelBuilder`. Nothing
here is a Rust transport.

What a `bazel build` command posts around the build, and buildx bypasses, is
posted from `src/engine/events.rs` as `BuildEvent`s implemented in Rust (the
protocol buffer is written with `axl-proto` and parsed into Bazel's Java class):

| Event | Content |
|---|---|
| `started` | uuid, start time, `build_tool_version` 9.0.1, command `build`, working and workspace directory, pid; announces everything below |
| `unstructured_command_line`, `structured_command_line` (`original`, `canonical`, `tool`) | `build`, `--buildx_script=file.star:main`, the configuration's settings, the options below and the targets |
| `options_parsed` | the same options as `--name=value` |
| `build_metadata` | the `metadata` of the sinks, plus `buildx=true` and `BUILD_TOOL=buildx` |
| `workspace_status` | Bazel's own `BuildInfoEvent`, posted by the workspace status action with every stable and volatile key (a stand-in with `BUILD_HOST`, `BUILD_USER` and `BUILD_TIMESTAMP` only when the action did not run) |
| `workspace` | the execution root |
| `pattern` | the targets of the build, announcing a `target_configured` each |
| `convenience_symlinks_identified` | empty: buildx makes no symlinks |
| `build_finished` | Bazel's own `BuildCompleteEvent`: exit code `SUCCESS`, `BUILD_FAILURE`, `ANALYSIS_FAILURE` or `PARTIAL_ANALYSIS_FAILURE`, the first failure as the failure detail, `last_message` set by the streamer on the last event |
| `build_metrics` | see below: runners, action cache statistics, timing, packages and targets of this build |
| `build_tool_logs` | `elapsed time` and a `targets` summary |

Settings are the only place option-style text appears in a script: each is an
option event `name=value` (`--name=value` in `options_parsed`), carrying the
effect and metadata tags of the generated options schema, as a UI expects.
buildx has no flags.

### The command line a UI reads

BES consumers infer how a build ran from its command line and its metrics, and
buildx has no `--remote_cache`: the cache is a value passed to `cached`. So the
command line events state what the build used, in Bazel's spelling. **These are
not flags buildx accepts**; they only describe the build.

| Option | When |
|---|---|
| `remote_cache=<endpoint>` | a `cached(store, ...)` is reachable; the endpoint as written (`grpcs://host:port`) |
| `remote_executor=<endpoint>` | a `remote(...)` is reachable |
| `remote_instance_name=<instance>` | the store or remote has one |
| `remote_accept_cached=true`, `remote_upload_local_results=true` | `cached` looks results up and stores local ones |
| `remote_download_outputs=all` | any of the above: outputs of remote actions and cache hits are downloaded as the action finishes, so this is what happens |
| `remote_cache_header=<name>=REDACTED`, `remote_exec_header=<name>=REDACTED` | the store sends headers: only the name is shown |
| `bes_backend`, `bes_results_url`, `bes_instance_name` | per BES sink |
| `build_metadata=KEY=VALUE` | per `metadata` entry of a BES sink |
| `bes_header=<name>=REDACTED` | per BES header |
| `build_event_binary_file=<path>` | per file sink |

The executor value is walked: `cached(store, inner)` counts as a cache and then
its `inner`; every executor in a `route`'s `executors` dict counts, whether or
not a spawn was sent there. A header value never appears in any event. Real
Bazel 9.0.1 does not redact `--remote_header`, `--remote_cache_header` or
`--bes_header` in its command line events at all (their values are in its BEP as
typed; only a `run` residue is shown as `REDACTED`), so the word is borrowed and
the value dropped. Options of the structured command line are `--name=value` in
`combined_form` (config settings keep `name=value`); the tags of the remote
and BES options are empty, since they are not in the generated schema.

### Build metrics

`build_metrics` states what happened in this `ctx.build`, from Bazel's own
action cache statistics where it can be read without a `CommandEnvironment`
(`MetricsCollector` is package private and takes one) and from counters on the
spawn strategy otherwise:

| Field | Source |
|---|---|
| `action_summary.actions_executed`, `runner_count` `total` | the action cache's misses, which is what Bazel's executed count equals |
| `runner_count` `remote cache hit` (kind `Remote`) | spawns answered by `cached` or by a server's cached result; they are not also `local` |
| `runner_count` `remote` (kind `Remote`) | spawns a remote executor ran |
| `runner_count` `local` (kind `Local`) | spawns Bazel's local runner ran (Bazel on macOS names its default runner `darwin-sandbox`; buildx has no sandbox) |
| `runner_count` `internal` | executed actions without a spawn (the workspace status action, file writes and the like) |
| `action_data` | spawns executed per mnemonic, and `BazelWorkspaceStatusAction` once when it ran |
| `action_cache_statistics` | `ActionCache.mergeIntoActionCacheStatistics`: hits, misses and the miss reasons |
| `actions_created` | actions enqueued, which in a new process is what analysis created |
| `timing_metrics` | wall time; CPU time of this process (not its children); analysis phase (start to the first action) and execution phase |
| `package_metrics.packages_loaded` | the package manager's statistics |
| `target_metrics` | requested targets resolved and configured (not their dependencies) |

The counters are per `ctx.build`, atomic, and a spawn is counted once. The
workspace status action runs in every build and counts as one executed action
without a spawn, so `total` and `internal` match Bazel's for the same build:
three genrules, cold, are `total` 4 and `internal` 1 in both, and a no-op build
is `total` 1, `internal` 1 (Bazel: "1 process: 3 action cache hit, 1 internal").

A run of one failing target posts the same event kinds as `bazel build
--build_event_binary_file` on the same workspace, except one `configuration`
event (and the progress event that links it): Bazel posts the baseline
configuration from `BuildView` before analysis, and buildx has none to post; no
event refers to it.

Still missing: test events (buildx does not run tests), fetch events, the
baseline `configuration` event, stdout and stderr text in progress events (except
the artifact upload failures described below), `NoBuildEvent` (a build always ends
with `build_finished`; an empty target list is an error before any event), and the
memory, artifact, build graph, worker, network and dynamic execution parts of
`build_metrics`.

BEP is a DAG whose events arrive in the order Bazel's threads post them, so
under load a child can arrive before its parent. Real Bazel does too (10,000
targets: the first out-of-order event is event 465). `buildx::bep::check_graph`
checks the strict order; `check_complete` checks what a tolerant consumer needs:
`started` first, every event announced exactly once and arrived, `last_message`
only on the last event, `build_finished` present. `examples/bep_dump.rs` prints
both for a file.

### URIs

The URI of a file in the stream never depends on whether a remote cache or
executor is configured. By default every referenced file gets a local
`file://` URI with its digest and length (Bazel's own
`LocalFilesArtifactUploader`, which never rewrites), also when the same build
runs on `ctx.execution.cached(store, ...)`. The tests compare the named sets of
a build with the cache and one without: equal.

`ctx.events.bes(..., artifacts = store)` (a `ctx.execution.store(...)`) is the
opt-in: a `BuildEventArtifactUploader` written in Rust over the store uploads each
referenced file the store lacks (files only, up to 8 at a time, each path once per
build) and the stream names it `bytestream://host:port/<instance>/blobs/<hash>/<size>`.
Directories, symlinks and files that failed to upload keep their `file://`
URI. A failed upload does not fail the build: it is counted, shown in the
stream's progress events and summarized on stderr when the build ends. A file is
read whole into memory to hash and upload it.

### Failures

A sink that cannot be set up, a server that refuses calls or drops the stream,
a certificate the JVM does not trust: the build completes and returns the same
results, and stderr says `buildx: events: bes grpc://host:port: <reason>` once.
Bazel retries first (4 attempts with backoff, so a refusing server delays the end
of the script by about 20 seconds). There is no option to make a BES failure fail
the build.

`grpcs://` is verified by the JVM, against its trust store, not by the store's
rustls roots: a private CA goes in with
`JAVA_TOOL_OPTIONS=-Djavax.net.ssl.trustStore=...`.

### Fake BES, examples and numbers

`examples/fake_bes.rs` (`cargo run -p buildx --example fake_bes -- [--port N]
[--refuse] [--drop-after N] [--save file.bep] [--tls cert.pem key.pem]`) is an
in-memory Build Event Service, a test fixture and not a product: it acks every
ordered event of a stream, records lifecycle events, stream events and header
names and values, and serves counters as JSON over HTTP on the port it prints.
`examples/bes-file.star`, `bes-grpc.star` and `bes-artifacts.star` say how to run
them, `examples/bep_dump.rs` reads a file.

On a scratch copy of `examples/large_bes` (10,000 genrules, 100 `sh_binary`,
10,102 targets, 40,427 events, 6.2 MB), release build, cold, three runs each:

| Sinks | Wall time | Peak RSS |
|---|---|---|
| none | 28.9 to 33.9 s | 460 to 510 MB |
| file | 32.1 to 34.8 s | 460 to 480 MB |
| BES (fake, gRPC) | 31.9 to 38.2 s | 565 to 573 MB |
| file and BES | 31.4 to 36.1 s | 564 to 571 MB |
| BES with `artifacts` (one run) | 41.9 s | 778 MB |

About 1,300 events per second are delivered while the build runs; finishing the
stream after the last action took 35 to 43 ms, and the progress line is the same.
The builds finished with every sink, and none hung when the stream ended
(aspect-cli issue 919: the stream is the transports' own queue, not a pipe the
build writes into).

The ignored tests (`cargo test -p buildx --test bep -- --ignored`, they need a
Bazel 9.0.1 release in the buildx cache) run real builds through the sinks.

## Examples

`examples/config.star`, `analyze.star`, `build.star`, `analysis-failure.star`, `engine-half.star`, `route.star`, `remote.star`, `cached.star`, `remote-failure.star`, `remote-runfiles.star`, `bes-file.star`, `bes-grpc.star` and `bes-artifacts.star` run from a fixture
workspace (each says what it needs in its header: a `MODULE.bazel`, `//:hello`,
`//lib:lib` and a `defs.bzl` with `write_file`, `tree`, `run_env`, `fail_action`
and `string_flag` rules). `examples/query.star`, `attrs.star`, `deps.star`, `live.star` and `types.star`
run from a workspace directory with a `//lib` package (each says so in its
header); `examples/crate.star`, `repo-deps.star` and `external.star` run from
this repository's root. `repo-deps.star` prints counts for `//...`,
`deps(//:all)`, `deps(//crates/buildx:buildx)`, `rdeps`, `allpaths`,
`buildfiles` and `loadfiles`; `external.star` for `@crates//:all` (a module
extension's repository) and `@bazel_tools` by apparent and canonical name.
`types.star` prints the generated `type()` of a result set (`Set`), rule
(`Rule`), label (`Label`), package (`Package`), runtime (`BlazeRuntime`) and
attribute list (`ImmutableList`), then `==` on two separately obtained labels
(`True`) and a label used as a dict key.

## Output base

buildx runs in Bazel's own output base for the workspace, `<output user
root>/_bazel_<user>/<md5 of the workspace path>`, so everything Bazel laid down
on disk is reused: external repositories, the repository cache, the action
cache and lockfiles. In-memory Skyframe state belongs to whichever process built
it and is never shared.

Two guards make that safe. buildx takes the same exclusive lock on
`<output base>/lock` that the Bazel client holds during a command, for the
engine's lifetime, and a concurrent `bazel` invocation or a second buildx sees
the usual "another command holds the lock" with buildx's pid. And buildx refuses
to start while a Bazel server owns the base, because that server trusts its
in-memory graph to match the disk; `bazel shutdown` first.
