# Connect this sbt 2.x build to an Aspect remote cache

sbt 2.x speaks the same gRPC remote-cache protocol Bazel uses, so an Aspect deployment can serve cacheable sbt task outputs to CI and developers. This is a remote-cache integration: it does not run sbt under Bazel and does not require migrating to Bazel first.

Inspect the sbt version in `project/build.properties`, the plugin list, resolver and credentials policy, and any task-cache opt-outs before changing anything. Preserve the existing sbt version and resolver policy.

## Discover the deployment

```text
aspect auth configure <cache-host>
aspect auth status
```

The first command records the deployment's endpoints in `~/.aspect/config.json` and logs in; the second prints the cache host to use below. Do not pass `--default` unless this deployment is meant to become the machine-wide default for every repository.

## Configure sbt

Add the remote-cache plugin in `project/plugins.sbt`:

```scala
addRemoteCachePlugin
```

Set the endpoint and its authentication headers. `remoteCache` takes the gRPC URI; `remoteCacheHeaders` takes `key=value` strings sent with every request. Read the token from the environment at invocation time rather than committing it:

```scala
Global / remoteCache := Some(uri("grpcs://<cache-host>"))
Global / remoteCacheHeaders := Seq(sys.env.getOrElse("SBT_CACHE_AUTH", ""))
```

`aspect get` prints `{"headers":{"Authorization":["Bearer …"]}}` for a given URI, so the developer's existing login supplies the value:

```text
SBT_CACHE_AUTH="Authorization=$(echo '{"uri":"https://<cache-host>"}' | aspect get | jq -r '.headers.Authorization[0]')"
```

Where the deployment authenticates with mutual TLS instead, use `remoteCacheTlsCertificate`, `remoteCacheTlsClientCertificate`, and `remoteCacheTlsClientKey`. Never commit a token, client key, or private certificate.

An unreachable or misconfigured endpoint does not fail the build: sbt logs the gRPC error and compiles locally. That is friendlier than Bazel's behaviour and it also means a silently broken cache looks exactly like a working one, so verify explicitly.

## Verify

Validate a representative `compile` or `test` task from a **clean second checkout**. Two things will otherwise serve the result and prove nothing: the project's own `target/` directory, and sbt's machine-global local cache. Isolate the second with `-Dsbt.global.localcache=<empty-dir>`.

The check that actually demonstrates the remote is a three-way comparison over the same source: build once with the remote enabled to populate it, then build a fresh copy with an empty local cache and the remote enabled — which should log no `compiling` line — and confirm the control, a fresh copy with an empty local cache and the remote disabled, does compile. Without that third run you cannot tell a remote hit from a local one.

Report which tasks are actually cacheable. A remote cache cannot improve a task whose inputs or outputs are non-hermetic, and sbt builds commonly contain several: tasks reading absolute paths, wall-clock time, environment variables, or Git state. Naming the cacheable set is the useful deliverable; a global speedup claim is not.

## Uploading

Default to letting developers populate the cache — a repository whose CI does not yet push entries has no other way to fill it, and a read-only cache returns nothing. Once CI populates the cache, or if the deployment is shared with other teams, restrict uploads to CI so a developer's local environment cannot poison shared entries. State which policy the repository chose.

Whether a given identity may upload is enforced by the deployment, not by the client. Refer questions about who may upload to whoever operates it, and do not probe for denial by attempting an upload against a shared cache.

Leave every change in the working tree: do not commit, branch, or push. Report the exact repository changes, the CI and developer authorization model, and the rollback path: remove the plugin line and the `remoteCache` settings, and `aspect auth remove <deployment>`.
