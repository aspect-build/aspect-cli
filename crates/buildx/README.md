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

Starlark values for Bazel's classes are generated, not reflected: with
`starlark = true` in `bindings.toml`, `blazon` emits a Starlark type per bound
class (`Rule`, `Label`, `Package`, `BlazeRuntime`, ...), registered once by
`register()`, and `type(x)` is the generated name. Classes without
`@StarlarkBuiltin` get their members from the spec: `@getRuleClass` is the
attribute `rule_class`, `getAttr(Ljava/lang/String;)` the method `getAttr`.
Query results and packages reach the script through `blazon_starlark::wrap`,
which picks the most specific generated type, so a `Set<Target>` iterates as
`Rule` and `InputFile` values.

Spec members resolve through the class, its superclasses and interfaces, so
`label`, `name`, `getAttr` and `visibility` declared on `RuleOrMacroInstance`
reach `Rule`, and `Packageoid.targets` reaches `Package`. A `Target` kind that
no spec entry names (`OutputFile`, `PackageGroup`, `InputFile`) shows up as
the nearest bound ancestor, so list `@getTargetKind` for each kind a script
may see.

## Examples

`examples/query.star`, `attrs.star`, `deps.star`, `live.star` and `types.star`
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
