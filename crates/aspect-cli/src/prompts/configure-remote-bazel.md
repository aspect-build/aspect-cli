# Connect this Bazel repository to an Aspect remote cache and BES

Inspect the existing `.bazelrc` files, CI configuration, and any current remote cache, remote executor, or BES endpoint before changing anything. Preserve working local developer commands.

## Discover the deployment

One command per machine records the deployment's endpoints and runs the interactive login:

```text
aspect auth configure <cache-host>
```

It fetches `/.well-known/oauth-protected-resource` from that host, writes the result to `~/.aspect/config.json`, and logs in. Take `<cache-host>` from the deployment's own documentation. Do not pass `--default` unless the developer intends this deployment to become the machine-wide default for every repository; name it with `--name <deployment>` and select it per-invocation instead. Confirm the recorded endpoints with `aspect auth status`, which prints the cache, BES, exec, and results-UI hosts.

## Configure the repository

Both paths below are supported. Configure the Bazel flags regardless: they are what makes the repository work for a developer who has not installed the Aspect CLI, and they document the endpoints in-tree.

Bazel flags, committed to `.bazelrc`. `aspect get` implements Bazel's credential-helper protocol, so the CLI supplies the bearer token to plain `bazel`:

```text
common:remote --remote_cache=grpcs://<cache-host>
common:remote --bes_backend=grpcs://<bes-host>
common:remote --bes_results_url=https://<results-host>/i/
common:remote --credential_helper=<cache-host>=aspect
```

Keep this on an opt-in `--config=remote` rather than `common`. An expired or missing credential makes `--remote_cache` fail the build outright at the capabilities query (`UNAUTHENTICATED`); it does not degrade to a local build. Tokens are short-lived, so an unconditional `common` line turns every expired login into a broken workspace. Add `--remote_local_fallback` if the repository prefers a slow build to a failed one.

For the Aspect CLI path, persist the deployment in `.aspect/config.axl` so it survives a clone instead of living in each developer's shell history:

```python
def config(ctx):
    for task in ["build", "test"]:
        ctx.tasks[task].args.deployment = "<deployment>"
        ctx.tasks[task].args.remote = "cache,bes"
```

`aspect wrapper install` writes a `tools/bazel` hook so Bazelisk routes `bazel build` through the CLI. The wrapper alone wires no remote: it needs the `config.axl` above, or explicit `--remote --deployment=<deployment>` flags.

Use `aspect describe '<task>'` for the resolved flag surface of any task, and `aspect build --announce-bazel-command=true` to print the exact `bazel` invocation the CLI constructs.

## Uploading

Default to letting developers populate the cache:

```text
common:remote --remote_upload_local_results
common:ci --remote_download_outputs=minimal
```

A repository whose CI does not yet run Bazel has no other way to fill the cache, and a read-only cache returns no hits at all — the first build is slower, not faster, and stays that way. Once CI populates the cache, or if the deployment is shared with other teams, reverse this: set `--noremote_upload_local_results` for developers and grant upload to CI alone, so a developer's local toolchain cannot poison shared entries.

State which policy the repository chose, and why, when reporting the change.

## Verify

A build that passes proves nothing: a warm `--disk_cache` serves the same actions and reports success. Verify against a cold cache, and report the numbers:

```text
bazel build --config=remote --disk_cache= --output_base=$(mktemp -d) //<target>
```

Read Bazel's `N processes:` line. A working remote reports `remote cache hit` entries once the cache holds this repository's actions; a first run against an empty cache reports none, which is expected and should be reported as such rather than as a failure. Confirm BES separately: the invocation prints a results URL, and `Streamed N build events` on completion.

Repository rules — dependency downloads, toolchain fetches, wheel and Maven resolution — are not action-cached, so they are unaffected by this configuration. On a repository whose cold build is dominated by fetching, say so rather than reporting the remote cache as a general speedup.

## Authorization

Whether a given identity may upload cache entries is enforced by the deployment, not by the client: a client-side `--noremote_upload_local_results` is a convention a developer can override, not a permission boundary. Verify the developer read path from build accounting, and refer questions about who may upload to whoever operates the deployment. Do not probe for denial by attempting an upload against a shared cache.

Publishing build events is a separate permission from uploading cache entries. Developers commonly have the first and not the second; a local build that streams to BES is working as intended.

Never commit a bearer token, client key, or private certificate. Machine-specific values — an absolute credential-helper path, a personal endpoint override — belong in an ignored `.bazelrc.user` consumed by `try-import`, not in the committed `.bazelrc`.

Leave every change in the working tree: do not commit, branch, or push. Report the exact repository changes, the CI and developer authorization model, and the rollback path: remove the `.bazelrc` block, delete `.aspect/config.axl`, run `aspect wrapper uninstall`, and `aspect auth remove <deployment>`.
