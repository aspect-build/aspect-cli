# Connect this sbt 2.x build to an Aspect remote cache

sbt 2.x can store cacheable task results in a Bazel-compatible gRPC remote cache. This does not run sbt under Bazel or enable remote execution.

Confirm `project/build.properties` already selects sbt 2.x. Preserve the existing sbt version, resolvers, credentials policy, task definitions, and CI commands.

## Select the deployment

Run `aspect auth status --output=json` and use the deployment named by the user or repository. If several deployments qualify and none is named, ask which one to use. If it is not configured yet, run `aspect auth configure <deployment-host> --name <deployment>` without `--default`.

## Configure sbt

Add the bundled remote-cache plugin in `project/plugins.sbt`:

```scala
addRemoteCachePlugin
```

In a root `.sbt` file, configure the remote only when an authentication header is present:

```scala
lazy val aspectCacheHeaders =
  sys.env.get("SBT_CACHE_AUTH").map(_.trim).filter(_.nonEmpty).toSeq

Global / remoteCache :=
  aspectCacheHeaders.headOption.map(_ => uri("grpcs://<cache-host>"))
Global / remoteCacheHeaders := aspectCacheHeaders
```

Mint the header from the existing Aspect login; never commit it:

```text
export SBT_CACHE_AUTH="Authorization=$(echo '{"uri":"https://<cache-host>"}' | aspect get | jq -r '.headers.Authorization[0]')"
```

The sbt server inherits this variable when it starts. After exporting or renewing it, run `sbt shutdown` before the next build so a new server receives the value.

## Verify

Use a representative `compile` task. Populate the remote once, then run the same source from a fresh checkout with an empty `-Dsbt.global.localcache=<dir>` and confirm it does not compile. As a control, repeat from another fresh checkout and empty local cache with `SBT_CACHE_AUTH` unset and confirm it does compile. Shut down the sbt server between changes to the environment.

Do not audit or change task cacheability, test semantics, CI policy, or deployment permissions as part of this task. Leave repository changes in the working tree without committing, branching, or pushing. Report the changed files and the rollback: remove the plugin line and remote-cache settings; remove the Aspect deployment only if this task added it.
