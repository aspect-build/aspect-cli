# Connect this sbt 2.x build to an Aspect remote cache

sbt 2.x speaks the same gRPC remote-cache protocol Bazel uses, so an Aspect deployment can serve cacheable sbt task outputs to CI and developers. This is a remote-cache integration: it does not run sbt under Bazel and does not require migrating to Bazel first.

Inspect the sbt version in `project/build.properties`, the plugin list, resolver and credentials policy, and any task-cache opt-outs before changing anything. Preserve the existing sbt version and resolver policy.

## Discover the deployment

Inspect the configured deployments before changing machine-level authentication state:

```text
aspect auth status --output=json
```

Use the deployment named by the user or repository. If none is named, use the sole logged-in deployment that advertises a cache endpoint; if several qualify, ask which one to use before changing anything. If the deployment is not configured yet, obtain one of its hosts from the user or its documentation, then run:

```text
aspect auth configure <deployment-host> --name <deployment>
```

This records the deployment's endpoints in `~/.aspect/config.json` and logs in. Do not pass `--default` unless this deployment is meant to become the machine-wide default for every repository. Record whether this task added the deployment: rollback must not remove a pre-existing or shared configuration.

## Configure sbt

Add the remote-cache plugin in `project/plugins.sbt`:

```scala
addRemoteCachePlugin
```

Set the endpoint and its authentication headers. `remoteCache` takes the gRPC URI; `remoteCacheHeaders` takes `key=value` strings sent with every request. Derive both settings from one validated environment value so a missing or malformed header disables the remote completely instead of creating a remote that fails every request:

```scala
lazy val aspectCacheHost = "<cache-host>"
lazy val aspectCacheAuthValue = sys.env.get("SBT_CACHE_AUTH").map(_.trim).filter(_.nonEmpty)
lazy val aspectCacheAuth = aspectCacheAuthValue.filter(header =>
  header.startsWith("Authorization=Bearer ") && header.length > "Authorization=Bearer ".length
)

Global / remoteCache := aspectCacheAuth.map(_ => uri(s"grpcs://$aspectCacheHost"))
Global / remoteCacheHeaders := aspectCacheAuth.toSeq

onLoadMessage := {
  val previous = onLoadMessage.value
  val status = aspectCacheAuthValue match {
    case None => s"[info] Aspect remote cache OFF ($aspectCacheHost): SBT_CACHE_AUTH is not set."
    case Some(_) if aspectCacheAuth.isEmpty =>
      s"[warn] Aspect remote cache OFF ($aspectCacheHost): expected Authorization=Bearer ..."
    case Some(_) => s"[info] Aspect remote cache ON ($aspectCacheHost)."
  }
  s"$previous\n$status"
}
```

`aspect get` prints `{"headers":{"Authorization":["Bearer …"]}}` for a given URI, so the developer's existing login supplies the value:

```text
export SBT_CACHE_AUTH="Authorization=$(echo '{"uri":"https://<cache-host>"}' | aspect get | jq -r '.headers.Authorization[0]')"
```

The variable is inherited when the sbt server starts, not afresh for every thin-client invocation. After exporting or renewing it, run `sbt shutdown` before the next build so a new server inherits the value; `reload` is not sufficient. Tokens are short-lived, so prefer a shell function that mints the header, shuts down any old server, and starts the intended task rather than placing a token in shell startup files.

Where the deployment authenticates with mutual TLS instead, use `remoteCacheTlsCertificate`, `remoteCacheTlsClientCertificate`, and `remoteCacheTlsClientKey`. Never commit a token, client key, or private certificate.

An unreachable or misconfigured endpoint does not fail the build: sbt logs the gRPC error and compiles locally. That is friendlier than Bazel's behaviour and it also means a silently broken cache looks exactly like a working one, so verify explicitly.

## Verify

Validate a representative `compile` or `test` task from a **clean second checkout**. Three things will otherwise preserve the first run's state and prove nothing: the project's own `target/` directory, sbt's machine-global local cache, and a live sbt server holding the first run's settings and environment. Give every arm its own empty `-Dsbt.global.localcache=<empty-dir>` and checkout, and run `sbt shutdown` before changing `SBT_CACHE_AUTH` or the remote configuration.

The check that actually demonstrates the remote is a three-way comparison over the same source: build once with the remote enabled to populate it; shut down that checkout's server; build a fresh copy with an empty local cache and the remote enabled, which should log no `compiling` line; then confirm the control, a fresh copy with an empty local cache and the remote disabled, does compile. Confirm the ON or OFF load message in every arm. Without the third run you cannot tell a remote hit from a local one.

In sbt 2.x, `test` is incremental and cacheable. A remote hit can legitimately report zero tests executed because the successful result was restored. Inspect CI for test-count gates, coverage collection, flaky-test detection, or any requirement to execute every test on every run. If fresh execution is required, use the uncached `testFull` task and report that decision; do not silently replace one with the other.

Report which tasks are actually cacheable. A remote cache cannot safely improve a task whose inputs or outputs are non-hermetic, and sbt builds commonly contain several: tasks reading absolute paths, wall-clock time, environment variables, or Git state; tasks with undeclared file outputs; and tasks whose required behaviour is a side effect. Restoring a cached task returns its saved value without executing the task body. Check suspicious generators and packaging steps by changing an input, confirming the output changes, reverting the input, and confirming restoration in both directions. Use `Def.declareOutput` for cacheable file outputs and `Def.uncached` for behaviour that must run every time. Naming the cacheable set and any deliberately uncached tasks is the useful deliverable; a global speedup claim is not.

## Uploading

Once `remoteCache` is configured, sbt 2.x has no repository-side push/pull or read-only switch. Whether an identity may upload is deployment policy, not a choice encoded by these settings. Inspect which CI and developer identities use the deployment and report the effective authorization model. If CI cannot upload, note that developers are the only possible populators; if the deployment is shared or CI already populates it, recommend restricting uploads to CI at the deployment.

Refer questions or changes about upload permission to whoever operates the deployment, and do not probe for denial by attempting an upload against a shared cache. Do not claim that the repository selected an upload policy when no repository setting expresses one.

Leave every change in the working tree: do not commit, branch, or push. Report the exact repository changes, the CI and developer authorization model, and the rollback path: remove the plugin line and the `remoteCache` settings. Run `aspect auth remove <deployment>` only if this task added that deployment and it is not shared with other repositories; otherwise leave the existing machine configuration intact.
