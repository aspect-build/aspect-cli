# `tools/bazel` wrapper — transparent Aspect routing

> **Source of truth:** <https://github.com/aspect-build/aspect-cli/blob/main/tools/bazel> (and this doc at `tools/bazel.md` alongside it). Both files are designed to be **vendored** into your workspace via the install instructions below — your `tools/bazel` and `tools/bazel.md` are copies of the upstream files. When upstream changes (routing tweaks, doc updates), re-run the vendor step to pull the latest.

A drop-in `tools/bazel` shell script that lets developers keep typing `bazel` while still reaching Aspect-specific functionality (`lint`, `format`, `delivery`, …) and Aspect's value-added wrappers around `build` / `test`.

Copy `tools/bazel` from this repo into your own workspace, adjust the verb lists at the top, commit it, and you're done. Bazel's flags need no maintenance here: the wrapper forwards them untouched.

## About the Aspect CLI

The [Aspect CLI](https://github.com/aspect-build/aspect-cli) (`aspect`) is a free, open-source, Apache-2.0-licensed task runner that extends Bazel with first-class developer workflows — built-in tasks for `build`, `test`, `run`, `format`, `lint`, `gazelle`, and `delivery`, plus custom tasks defined in [AXL](https://aspect.build/docs/cli/overview#aspect-extension-language) (Aspect Extension Language, typed Starlark). Same command locally and in every CI provider; native integration with GitHub Status Checks, Buildkite Annotations, and the equivalents on GitLab and CircleCI.

- **Docs:** <https://aspect.build/docs/cli/overview>
- **Source / releases:** <https://github.com/aspect-build/aspect-cli>
- **Install:** `curl -fsSL https://install.aspect.build | bash`

### Minimum Aspect CLI version

**Wrapper version 2 requires an Aspect CLI with Bazel flag passthrough.** It forwards your arguments verbatim and relies on `aspect` to forward the flags it doesn't recognize to Bazel; on an older CLI those flags fail the parse (`error: unexpected argument`). Version 1, which rewrote them into `--bazel-flag=…`, remains the choice for an older CLI.

Version 2 also renames the verb list `ASPECT_VERBS_WITH_BAZEL_FLAGS` → `ASPECT_VERBS`, since nothing rewrites flags any more. Re-apply any edits you made to it under the new name when you re-vendor.

Independently, **any version of this wrapper requires Aspect CLI v2026.23.18 or newer** for the `ASPECT_CLI_RUNNING` re-entry signal it relies on to avoid infinite recursion when `aspect` shells back out to `bazel` (see [How it avoids infinite recursion](#how-it-avoids-infinite-recursion) below). Older versions don't set it, and the wrapper would route every internal `bazel` child invocation back through `aspect` indefinitely.

## What it does

The `tools/bazel` hook is a [Bazelisk](https://github.com/bazelbuild/bazelisk) feature — the real `bazel` binary does not look for it. When Bazelisk finds `tools/bazel` in your workspace it execs that script instead of the bazel version it resolved, passing the resolved path as `$BAZEL_REAL`. (So this only works if the `bazel` on your `PATH` is Bazelisk — the standard setup.) This wrapper uses that hook to dispatch each command to the right tool:

| You type | What runs |
| --- | --- |
| `bazel build //... --keep_going --config=ci` | `aspect build //... --keep_going --config=ci` |
| `bazel build -c opt //...` | `aspect build -c opt //...` |
| `bazel test //... --test_output errors` | `aspect test //... --test_output errors` |
| `bazel --output_base /tmp/o build //...` | `aspect --output_base /tmp/o build //...` |
| `bazel lint --config=ci //src/...` | `aspect lint --config=ci //src/...` |
| `bazel delivery --config=release //...` | `aspect delivery --config=release //...` |
| `bazel query 'deps(//foo)'` | `$BAZEL_REAL query 'deps(//foo)'` (vanilla bazel, unchanged) |
| `bazel info workspace` | `$BAZEL_REAL info workspace` |
| `bazel my-custom-task //...` | `aspect my-custom-task //...` (unknown verb → aspect verbatim) |

The interesting case is the verbs aspect wraps (`build` / `test`) and the aspect verbs that drive Bazel internally (`lint`, `format`, `delivery`, …). The wrapper routes those through `aspect` so you pick up its DX improvements (artifact upload, GitHub PR comments, BES streaming, …), and **bazel-native flags keep working** — the arguments reach `aspect` exactly as you typed them, and aspect forwards the flags it doesn't recognize to Bazel itself.

> **Why `run` isn't wrapped by default:** `aspect run` exists, but its semantics don't yet line up closely enough with `bazel run` to shadow it transparently. Until that's resolved, `bazel run` goes to vanilla bazel (via `BAZEL_VERBS`). Reach for `aspect run` directly when you want the aspect behavior, or add `run` to `ASPECT_VERBS` in your repo copy once you've validated it for your workflows.

## How verb routing works

The wrapper decides where a command goes from two lists at the top of the script:

- `ASPECT_VERBS` — verbs routed to `aspect` (default `build buildifier delivery format gazelle lint test`). These are also the verbs that may fall back to vanilla bazel when `aspect` isn't installed.
- `BAZEL_VERBS` — the closed set of Bazel commands. A verb here that's *not* in the list above (`query`, `info`, `clean`, `mod`, `coverage`, …) goes to vanilla bazel.

The rules, in order (`ASPECT_WRAPPER_SKIP=1` short-circuits all of them — see below):

1. Verb in `ASPECT_VERBS` → `aspect <verb>`, arguments forwarded verbatim.
2. Verb in `BAZEL_VERBS` but not the above (`query`, `info`, `clean`, `mod`, `coverage`, …) → vanilla bazel, untouched.
3. Any other verb → `aspect <verb>`, arguments forwarded verbatim. This is how custom `.axl` tasks (arbitrary names) reach aspect. The only thing rule 1 adds is the vanilla-bazel fallback when `aspect` isn't installed, which only makes sense for verbs Bazel also has.

### When `aspect` isn't installed

The wrapper's presence means the org wants devs on aspect, so if a command would route to aspect (rule 1 or 3) but `aspect` isn't on `PATH`, the wrapper prints install instructions:

```
curl -fsSL https://install.aspect.build | bash
```

…or see <https://aspect.build/docs/cli/install>. If the verb is also a real Bazel command (`build`/`test`) the wrapper then falls back to vanilla bazel so the command still runs; for aspect-only verbs (`lint`, `format`, custom tasks) Bazel has nothing to run, so it exits. Plain bazel verbs (rule 2) never need aspect and run regardless.

## How flags are handled

They aren't. Every argument is forwarded to `aspect` exactly as typed, in the position you typed it.

Aspect accepts a Bazel flag directly: a flag it doesn't recognize is forwarded to Bazel in the slot it appeared in — before the verb it becomes a Bazel *startup* option, after the verb a *command* option, mirroring how `bazel` itself splits its command line. Bazel's own spellings all work (`--config=ci`, `--config ci`, `-c opt`, `--jobs 8`), and `--` still ends flag parsing so hyphen-led target patterns and `run` arguments pass through untouched.

The wrapper keeps one small list, `BAZEL_STARTUP_VALUE_FLAGS` (16 entries), and it is not about flags reaching Bazel: verb detection has to know that `bazel --bazelrc build query …` means the *query* command, not `build`. Nothing else about Bazel's flags is embedded, so a new Bazel flag needs no change here. It also means a **newer wrapper needs a newer CLI**: see [Minimum Aspect CLI version](#minimum-aspect-cli-version). On an older CLI, a bare Bazel flag comes back as `error: unexpected argument '--keep_going' found`; wrap it as `--bazel-flag=--keep_going` (still supported) or upgrade.

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

When the wrapper routes a command **through aspect**, it prints a single grey trace line on stderr beforehand showing the resolved command. This makes the routing visible:

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

…and the wrapper forwards **everything** straight to `$BAZEL_REAL`, untouched — no verb parsing, no list checks. It's a complete escape hatch: even aspect-only verbs like `lint` go to vanilla bazel (where they'll error if bazel has no such command — which is the point; you asked for vanilla bazel).

This is per-shell, per-developer. To change routing repo-wide instead, edit the verb lists (e.g. remove `build test run` from `ASPECT_VERBS` so they fall through to the vanilla-bazel branch).

## How it avoids infinite recursion

When `tools/bazel` routes a verb through `aspect`, aspect then needs to spawn its own child `bazel` to do the actual work. If your `PATH` still has `tools/bazel` first (the normal case), that child `bazel` invocation re-enters the wrapper — which routes it back to `aspect` — which spawns `bazel` again — and so on forever.

The fix: aspect sets `ASPECT_CLI_RUNNING=1` on every child `bazel` it spawns. `tools/bazel` checks for this on entry and forwards straight to the real bazel (`$BAZEL_REAL` if set, else the next `bazel` on `PATH`) without any routing logic. The cycle is broken at the first hop. This matches the pattern Bazelisk uses for `BAZELISK_SKIP_WRAPPER`.

**Customers should never set `ASPECT_CLI_RUNNING` manually** — it's an implementation detail of the aspect ↔ wrapper handshake. If you wrap `aspect-cli` in your own wrapper, propagate the variable through.

## Customizing

Two lists at the top of the script drive every routing decision; edit them in your repo copy:

- `ASPECT_VERBS` — verbs routed to `aspect`. Default: `build buildifier delivery format gazelle lint test`. Add your own aspect commands here — including `run`, once you're ready for `aspect run` to shadow `bazel run` in your workspace. An unlisted verb also routes to aspect, so what listing one really buys is the vanilla-bazel fallback when `aspect` isn't installed. (`ASPECT_WRAPPER_SKIP=1` bypasses this entirely — everything goes to vanilla bazel.)
- `BAZEL_VERBS` — the closed set of Bazel commands. A verb here that's *not* in the list above forwards to vanilla bazel. A verb in *neither* list is treated as a custom aspect task and routed to aspect verbatim. Update this only if Bazel adds a command, and regenerate it from `bazel help completion`'s `BAZEL_COMMAND_LIST` rather than `bazel help` — the latter hides some commands (`config`), and a hidden command missing here gets misrouted to `aspect`:

  ```
  bazel help completion | sed -n 's/^BAZEL_COMMAND_LIST="\(.*\)"$/\1/p'
  ```

Plus `BAZEL_STARTUP_VALUE_FLAGS`, which exists only so a space-form startup option's value isn't mistaken for the verb; regenerate it with `bazel help startup_options` when Bazel adds one. Nothing else about Bazel's flags is embedded.

## What it deliberately doesn't do

- **It doesn't fetch or pin Bazel.** That's Bazelisk's job. The wrapper just decides which tool to exec.
- **It doesn't inspect your flags.** Every argument is forwarded verbatim, so a typo or a brand-new Bazel flag surfaces wherever it actually belongs — in aspect's parse error, or in Bazel's.
- **It doesn't replace `bazel`.** You still install Bazel through Bazelisk (which is what execs this script). The wrapper is purely an in-workspace dispatcher.
