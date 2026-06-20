# `aspect cache diff` example

A self-contained workspace for exercising `aspect cache diff` — a command that
lists the **test targets affected** by the current tree by mining a single
`--experimental_remote_require_cached` probe of the remote cache.

Strictly a **remote-cache** operation: no remote execution (RBE) required, and
no separate "seed" step. The probe stands up an in-process dummy executor only
so `--experimental_remote_require_cached` engages (it's a no-op with a cache
alone); require-cached then denies every cache miss before anything runs.

## Setup: a writable remote cache

The command reads from — and the baseline writes to — a remote cache. This
example's `.bazelrc` points at `grpc://127.0.0.1:9092`, so bring up a local cache
there (or swap in your own endpoint):

```bash
# any REAPI cache works; e.g. bazel-remote:
bazel-remote --dir /tmp/bazel-remote --max_size 5 --grpc_address 127.0.0.1:9092
```

No RBE endpoint is needed — just a cache you can write to.

## Graph

```
libshared ──► testA, testB     edit → both fall out
libA ───────► testA, testC     edit → testA + testC
libB ───────► testB            edit → testB only
```

## How the baseline gets into the cache

A cache hit means "unaffected", so the cache must already hold the action
results for the mainline tree. That happens as a byproduct of **normal CI** —
just run your tests with cache upload (here, do it once to seed the baseline):

```bash
bazel test //... --remote_upload_local_results
```

There is **no `--seed` and no RBE.** The one rule that makes it work:

> The baseline and the probe must use the **same Bazel flags**, so they compute
> the same action digests (cache keys). `aspect cache diff` resolves flags from
> the same rc/config your `bazel test` / `aspect test` uses, so they match
> automatically. (This example ships a minimal `.aspect/config.axl` for exactly
> this reason — to keep its flags identical on both sides. A flag that changes an
> action's environment, e.g. `--action_env=FOO`, must be present on *both* sides
> or neither.)

Seed on a **clean/fresh checkout** (or a clean `--output_base`): on a warm base
`bazel test` finds everything locally cached, executes nothing, and uploads
nothing. CI checkouts are cold, so this is automatic there.

## Run

```bash
cli2 cache diff                      # overreport (default)
cli2 cache diff --mode=precise
cli2 cache diff --format=json
cli2 cache diff --exec "bazel test"
cli2 cache diff //some/...           # scope with positional patterns
```

Clean tree ⇒ `Affected 0 of N`. Repeated runs are fine — the probe forces fresh
remote lookups with `--nokeep_state_after_build --nouse_action_cache`, so it runs
on the default output base with no separate or wiped base.

## Output contract

- **stdout** — affected test labels, one per line. Pipe it: `aspect cache diff | xargs bazel test`.
- **stderr** — progress, per-target reason (incl. the cache-missed target it was `caused by`), summary.
- `--format=json` — one document on stdout instead of bare lines.
- `--exec="<cmd>"` — run `<cmd> <labels…>` instead of writing stdout.

Bazel's own output is hidden (the expected require-cached denials are noise),
surfaced only on an unexpected failure.

## Try a change

```bash
sed -i '' 's/echo A-v1/echo A-v2/' BUILD.bazel
cli2 cache diff                      # → //:testA, //:testC (caused by //:libA)
sed -i '' 's/echo A-v2/echo A-v1/' BUILD.bazel    # revert
```

### overreport vs precise

`overreport` reverse-deps from cache-missed targets to dependent tests — runs
nothing, but flags a test even when its dependency would rebuild to an identical
output. `precise` first builds + uploads the missing non-test actions, then a
test is affected only if its own test action misses (a genuine input change).

**Execution guarantee:** *no test ever runs* in either mode (the probe is
require-cached; precise builds with `bazel build`, which can't execute
`TestRunner`). overreport executes **nothing**; precise executes only the missing
**non-test** actions. Exception: actions tagged `no-remote` / `local` /
`no-cache` bypass the remote path, so they run locally and read as affected.

> Shared-cache note: the `.bazelrc` here points at a remote cache. `precise`
> uploads, so on a shared cache a repeated edit to a fixed value can later read
> as a hit. Use fresh values for repeat demos, or a private/local cache.

## Requirements

- A writable remote cache (`--remote_cache` in `.bazelrc`). No RBE.
- The dev CLI: `cli2` (alias for `target/debug/aspect-cli`).
