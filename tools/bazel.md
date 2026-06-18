# `tools/bazel` wrapper — transparent Aspect routing

> **Source of truth:** <https://github.com/aspect-build/aspect-cli/blob/main/tools/bazel> (and this doc at `tools/bazel.md` alongside it). Both files are designed to be **vendored** into your workspace via the install instructions below — your `tools/bazel` and `tools/bazel.md` are copies of the upstream files. When upstream changes (new Bazel flags, routing tweaks, doc updates), re-run the vendor step to pull the latest.

A drop-in `tools/bazel` shell script that lets developers keep typing `bazel` while still reaching Aspect-specific functionality (`lint`, `format`, `delivery`, …) and Aspect's value-added wrappers around `build` / `test`.

Copy `tools/bazel` from this repo into your own workspace, adjust the verb lists at the top, commit it, and you're done.

## About the Aspect CLI

The [Aspect CLI](https://github.com/aspect-build/aspect-cli) (`aspect`) is a free, open-source, Apache-2.0-licensed task runner that extends Bazel with first-class developer workflows — built-in tasks for `build`, `test`, `run`, `format`, `lint`, `gazelle`, and `delivery`, plus custom tasks defined in [AXL](https://aspect.build/docs/cli/overview#aspect-extension-language) (Aspect Extension Language, typed Starlark). Same command locally and in every CI provider; native integration with GitHub Status Checks, Buildkite Annotations, and the equivalents on GitLab and CircleCI.

- **Docs:** <https://aspect.build/docs/cli/overview>
- **Source / releases:** <https://github.com/aspect-build/aspect-cli>
- **Install:** `curl -fsSL https://install.aspect.build | bash`

### Minimum Aspect CLI version

**This wrapper requires Aspect CLI v2026.23.18 or newer.** That release adds the `ASPECT_CLI_RUNNING` re-entry signal the wrapper relies on to avoid infinite recursion when `aspect` shells back out to `bazel` (see [How it avoids infinite recursion](#how-it-avoids-infinite-recursion) below). Older versions don't set `ASPECT_CLI_RUNNING`, and the wrapper would route every internal `bazel` child invocation back through `aspect` indefinitely.

## What it does

The `tools/bazel` hook is a [Bazelisk](https://github.com/bazelbuild/bazelisk) feature — the real `bazel` binary does not look for it. When Bazelisk finds `tools/bazel` in your workspace it execs that script instead of the bazel version it resolved, passing the resolved path as `$BAZEL_REAL`. (So this only works if the `bazel` on your `PATH` is Bazelisk — the standard setup.) This wrapper uses that hook to dispatch each command to the right tool:

| You type | What runs |
| --- | --- |
| `bazel build //... --keep_going --config=ci` | `aspect build //... --bazel-flag=--keep_going --bazel-flag=--config=ci` |
| `bazel build -c opt //...` | `aspect build --bazel-flag=-c=opt //...` |
| `bazel test //... --test_output errors` | `aspect test //... --bazel-flag=--test_output=errors` |
| `bazel lint --config=ci //src/...` | `aspect lint --bazel-flag=--config=ci //src/...` |
| `bazel format --keep_going` | `aspect format --bazel-flag=--keep_going` |
| `bazel delivery --config=release //...` | `aspect delivery --bazel-flag=--config=release //...` |
| `bazel query 'deps(//foo)'` | `$BAZEL_REAL query 'deps(//foo)'` (vanilla bazel, unchanged) |
| `bazel info workspace` | `$BAZEL_REAL info workspace` |
| `bazel my-custom-task //...` | `aspect my-custom-task //...` (unknown verb → aspect verbatim) |

The interesting case is the verbs aspect wraps (`build` / `test`) and the aspect verbs that drive Bazel internally (`lint`, `format`, `delivery`, …). The wrapper routes those through `aspect` so you pick up its DX improvements (artifact upload, GitHub PR comments, BES streaming, …), but **bazel-native flags keep working** — they're transparently rewritten as `--bazel-flag=<flag>` so aspect forwards them verbatim to Bazel.

> **Why `run` isn't wrapped by default:** `aspect run` exists, but its semantics don't yet line up closely enough with `bazel run` to shadow it transparently. Until that's resolved, `bazel run` goes to vanilla bazel (via `BAZEL_VERBS`). Reach for `aspect run` directly when you want the aspect behavior, or add `run` to `ASPECT_VERBS_WITH_BAZEL_FLAGS` in your repo copy once you've validated it for your workflows.

## How verb routing works

The wrapper decides where a command goes from two lists at the top of the script:

- `ASPECT_VERBS_WITH_BAZEL_FLAGS` — verbs routed to `aspect` **with** bazel-flag rewriting (default `build buildifier delivery format gazelle lint test`).
- `BAZEL_VERBS` — the closed set of Bazel commands. A verb here that's *not* in the list above (`query`, `info`, `clean`, `mod`, `coverage`, …) goes to vanilla bazel.

The rules, in order (`ASPECT_WRAPPER_SKIP=1` short-circuits all of them — see below):

1. Verb in `ASPECT_VERBS_WITH_BAZEL_FLAGS` → `aspect <verb>` with bazel flags rewritten (see below).
2. Verb in `BAZEL_VERBS` but not the above (`query`, `info`, `clean`, `mod`, `coverage`, …) → vanilla bazel, untouched.
3. Any other verb → `aspect <verb>`, args forwarded verbatim. This is how custom `.axl` tasks (arbitrary names) reach aspect; we don't rewrite their flags since they may define their own. **If your custom command accepts `--bazel-flag` / `--bazel-startup-flag`, add it to `ASPECT_VERBS_WITH_BAZEL_FLAGS`** so the wrapper rewrites bazel-native flags for it too (rule 1).

### When `aspect` isn't installed

The wrapper's presence means the org wants devs on aspect, so if a command would route to aspect (rule 1 or 3) but `aspect` isn't on `PATH`, the wrapper prints install instructions:

```
curl -fsSL https://install.aspect.build | bash
```

…or see <https://aspect.build/docs/cli/install>. If the verb is also a real Bazel command (`build`/`test`) the wrapper then falls back to vanilla bazel so the command still runs; for aspect-only verbs (`lint`, `format`, custom tasks) Bazel has nothing to run, so it exits. Plain bazel verbs (rule 2) never need aspect and run regardless.

## How flag rewriting works

Bazel is the closed set. The wrapper classifies each flag against the embedded Bazel flag lists; a flag whose name is in one of them is a Bazel flag, and **everything else** — aspect's own flags and anything unrecognized — passes through to aspect unchanged.

- `BAZEL_VALUE_FLAGS` — Bazel flags that take a value (consume the next token when space-separated). From `bazel help build|test|run|startup_options --long`, **Bazel 9.x**.
- `BAZEL_BOOL_FLAGS` — Bazel boolean (no-value) flags. The wrapper also matches their `--no…` negations.
- `BAZEL_SHORT_VALUE_FLAGS` (`-c`, `-j`) / `BAZEL_SHORT_BOOL_FLAGS` (`-k`, `-s`, `-t`) — the short-flag abbreviations, so `bazel build -c opt //...` consumes `opt` instead of leaving it as a stray target.

The rules:

1. A recognized Bazel flag is wrapped: `--keep_going` → `--bazel-flag=--keep_going`; `--config=ci` → `--bazel-flag=--config=ci`; `--config ci` → `--bazel-flag=--config=ci` (next token consumed and glued with `=`); `-c opt` → `--bazel-flag=-c=opt`.
2. Anything else passes through unchanged: aspect globals (`--task-key`, `--timing`), feature flags (`--artifact-upload:enabled`), and unrecognized flags — including a typo or a brand-new Bazel flag not yet in the lists, which simply goes to aspect until you add it.
3. After `--`, everything passes through verbatim (positional targets, `run` arguments, etc.).
4. Flags **before the verb** get the same classification, except Bazel flags wrap as `--bazel-startup-flag=…` (which aspect accepts as a post-verb flag, so they're moved after the verb). Aspect global flags stay in front of the verb.

To refresh the embedded Bazel flag lists when Bazel adds flags. These scrape `bazel help` and aren't aspect- or repo-specific — the output depends only on the Bazel version, so run them against the version you target. Note that in a workspace where this wrapper is on `PATH`, plain `bazel help build` would route through the wrapper; `ASPECT_WRAPPER_SKIP=1` (below) bypasses it so `help` reaches the real bazel. (Outside such a workspace, drop the prefix.)

```sh
export ASPECT_WRAPPER_SKIP=1  # ensure `bazel help` reaches the real bazel

# Value-taking flags
for h in build test run startup_options; do bazel help $h --long; done \
  | grep -oE '^[[:space:]]+--[a-z_][a-z0-9_]*' | grep -vE '\-\-\[no\]' \
  | sed -E 's/^[[:space:]]*--//' | sort -u

# Boolean flags
for h in build test run startup_options; do bazel help $h --long; done \
  | grep -oE '^[[:space:]]+--\[no\][a-z][a-z0-9_]*' \
  | sed -E 's/^[[:space:]]*--\[no\]//' | sort -u
```

## Installing in your repo

Grab the script straight from this repo and drop it in your own `tools/` directory, along with this doc so your team has the routing reference:

```sh
mkdir -p tools
base=https://raw.githubusercontent.com/aspect-build/aspect-cli/main/tools
curl -fsSL "$base/bazel" -o tools/bazel
curl -fsSL "$base/bazel.md" -o tools/bazel.md
chmod +x tools/bazel
git add tools/bazel tools/bazel.md
```

Make sure the `bazel` on your `PATH` is [Bazelisk](https://github.com/bazelbuild/bazelisk) — it's what execs `tools/bazel` (the real bazel binary doesn't). Every Bazelisk release since 2019 honors the hook. No other changes needed.

## Trace output

When the wrapper routes a command **through aspect**, it prints a single grey trace line on stderr beforehand showing the resolved command. This makes the rewrite visible:

```
[tools/bazel] aspect build --bazel-flag=--keep_going --bazel-flag=--config=ci //...
→ 🎬 Running `build` task
…
```

When the wrapper forwards **straight to bazel** (e.g. `bazel info`, `bazel query`, anything under skip mode below), the trace is silent — that path is uninteresting and would just add noise.

Env vars:

- `ASPECT_WRAPPER_TRACE=1` — print the trace on every exec, including bazel forwarding. Also forces the line on even when stderr is not a TTY (useful for piping debug output to a file).
- `ASPECT_WRAPPER_QUIET=1` — suppress the trace entirely. Wins over `TRACE`.

The trace is silent under non-TTY stderr by default, so CI logs and command-substitution captures aren't polluted.

## Skip mode — total bypass

Some developers prefer a 1:1 Bazel experience locally and reach for `aspect <verb>` directly when they want the wrapped behavior. Set:

```sh
export ASPECT_WRAPPER_SKIP=1
```

…and the wrapper forwards **everything** straight to `$BAZEL_REAL`, untouched — no verb parsing, no list checks, no flag rewriting. It's a complete escape hatch: even aspect-only verbs like `lint` go to vanilla bazel (where they'll error if bazel has no such command — which is the point; you asked for vanilla bazel).

This is per-shell, per-developer. To change routing repo-wide instead, edit the verb lists (e.g. remove `build test run` from `ASPECT_VERBS_WITH_BAZEL_FLAGS` so they fall through to the vanilla-bazel branch).

## How it avoids infinite recursion

When `tools/bazel` routes a verb through `aspect`, aspect then needs to spawn its own child `bazel` to do the actual work. If your `PATH` still has `tools/bazel` first (the normal case), that child `bazel` invocation re-enters the wrapper — which routes it back to `aspect` — which spawns `bazel` again — and so on forever.

The fix: aspect sets `ASPECT_CLI_RUNNING=1` on every child `bazel` it spawns. `tools/bazel` checks for this on entry and forwards straight to the real bazel (`$BAZEL_REAL` if set, else the next `bazel` on `PATH`) without any routing logic. The cycle is broken at the first hop. This matches the pattern Bazelisk uses for `BAZELISK_SKIP_WRAPPER`.

**Customers should never set `ASPECT_CLI_RUNNING` manually** — it's an implementation detail of the aspect ↔ wrapper handshake. If you wrap `aspect-cli` in your own wrapper, propagate the variable through.

## Customizing

Two lists at the top of the script drive every routing decision; edit them in your repo copy:

- `ASPECT_VERBS_WITH_BAZEL_FLAGS` — verbs routed to `aspect` with bazel-flag rewriting. Default: `build buildifier delivery format gazelle lint test`. Add your own bazel-flag-aware aspect commands (e.g. a custom task that shells out to bazel) — including `run`, once you're ready for `aspect run` to shadow `bazel run` in your workspace. (`ASPECT_WRAPPER_SKIP=1` bypasses this entirely — everything goes to vanilla bazel.)
- `BAZEL_VERBS` — the closed set of Bazel commands. A verb here that's *not* in the list above forwards to vanilla bazel. A verb in *neither* list is treated as a custom aspect task and routed to aspect verbatim. Update this only if Bazel adds a command.

Plus the embedded Bazel flag lists (`BAZEL_VALUE_FLAGS`, `BAZEL_BOOL_FLAGS`, `BAZEL_SHORT_VALUE_FLAGS`, `BAZEL_SHORT_BOOL_FLAGS`) covered above. There is no aspect-flag list — anything not recognized as a Bazel flag passes through to aspect.

## What it deliberately doesn't do

- **It doesn't fetch or pin Bazel.** That's Bazelisk's job. The wrapper just decides which tool to exec.
- **It doesn't second-guess unknown flags.** A flag not in the embedded Bazel lists passes through to aspect unchanged (rather than being force-wrapped), so a typo or a not-yet-listed flag surfaces wherever it actually belongs.
- **It doesn't replace `bazel`.** You still install Bazel through Bazelisk (which is what execs this script). The wrapper is purely an in-workspace dispatcher.
