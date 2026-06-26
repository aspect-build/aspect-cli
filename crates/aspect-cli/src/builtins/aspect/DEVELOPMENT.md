# Developing built-in tasks

This is the contributor guide for **`crates/aspect-cli/src/builtins/aspect/`** — the AXL module that ships with `aspect-cli` and provides the `build` / `test` / `lint` / `format` / `gazelle` / `delivery` tasks. If you only want to know what each task *does*, read [`README.md`](README.md) first.

This guide explains *why* the existing tasks look the way they do and how to keep new tasks consistent with them. It covers:

1. [Architecture at a glance](#architecture-at-a-glance)
2. [The per-task lifecycle](#the-per-task-lifecycle)
3. [Traits the framework exposes](#traits-the-framework-exposes)
4. [State management: patterns and rules](#state-management-patterns-and-rules) — **load-bearing for new features**
5. [BES streaming and the broadcaster race](#bes-streaming-and-the-broadcaster-race)
6. [Per-kind result libraries](#per-kind-result-libraries)
7. [Status checks and annotations](#status-checks-and-annotations)
8. [Artifact uploads](#artifact-uploads)
9. [Anatomy of a task `_impl`](#anatomy-of-a-task-_impl) — the canonical flow with annotated code
10. [How to add a new task](#how-to-add-a-new-task)
11. [How to add a new task kind (new renderer)](#how-to-add-a-new-task-kind-new-renderer)
12. [Testing](#testing)

---

## Architecture at a glance

```
                        ┌───────────────────────────────┐
   user command  ───►   │  task `_impl(ctx)`            │
                        │  e.g. build.axl, lint.axl     │
                        └─────────────┬─────────────────┘
                                      │ iterates
                                      ▼
              ┌─────────────────────────────────────────────────┐
              │ trait hook lists                                 │
              │   bazel_trait.build_start / build_event /        │
              │     build_end / build_event_sinks / task_flags   │
              │   hc_trait.health_check                          │
              │   lifecycle.task_update                          │
              └────────────┬─────────────────┬──────────────────┘
                           │                 │
              registered by │                 │ consumed by
                           ▼                 ▼
        ┌──────────────────────────────────────────────┐
        │ features: Workflows, ArtifactUpload,         │
        │           GithubStatusChecks,                │
        │           GithubStatusComments,              │
        │           GithubLintComments,                │
        │           BuildkiteAnnotations, …            │
        └────────────┬─────────────────────────────────┘
                     │ render via dispatch table in
                     ▼
        ┌──────────────────────────────────────────────┐
        │ private/lib/check_dispatch.axl                       │
        │   RENDERERS = {bazel, lint, format,          │
        │                gazelle, delivery}            │
        └────────────┬─────────────────────────────────┘
                     │ each entry points at
                     ▼
        ┌──────────────────────────────────────────────┐
        │ private/lib/<kind>_results.axl                       │
        │   init_data() / render_check_output()     │
        └──────────────────────────────────────────────┘
```

Tasks own the *flow* (which Bazel command runs when, what extra processing happens around it). Features own the *cross-cutting concerns* (status checks, annotations, artifact uploads, runner health). Result libraries own the *rendering*. Tasks never poke at GitHub or Buildkite directly — they fire lifecycle events and the features react.

---

## The per-task lifecycle

`TaskLifecycleTrait` (defined in [`private/lib/lifecycle.axl`](private/lib/lifecycle.axl)) has a **single** slot, **`task_update(ctx, TaskUpdate)`**, fired zero or more times during the task. The *first* and *last* updates carry extra meaning, so one hook covers the whole lifecycle:

- **First update → init.** A handler's first `task_update` is its cue to initialize: authenticate, create the GitHub check run / first BK annotation, and read `update.subject` for the rendered title. `setup_phase` (see below) emits this first update — the `🔧 Setup` phase mark — at the very start of every task, so init always lands inside the Setup phase. Each handler guards init with a private `_state["_initialized"]` flag.

- **Middle updates → progress.** Tasks emit running updates as they make progress (lint emits one per SARIF report; build/test emit on every BES event that updates the failure / test-summary state). The per-surface throttle in `private/lib/check_dispatch` collapses chatty updates.

- **Last update → terminal.** The producer's final emit carries `final = True` with a terminal `status` (`"passed"`, `"failed"`, `"warning"`, `"aborted"`). Features take that as the cue to complete the check run / finalise the annotation, and the throttle always lets `final` through. `task_update` returns a `TaskConclusion` on the `final` emit for `_impl` to return.

Each `TaskUpdate` carries:
- `kind` — the result-library identifier (e.g. `"lint_results"`). Drives renderer dispatch.
- `status` — `"running"`, `"failing"`, `"passed"`, `"failed"`, `"warning"`, or `"aborted"`.
- `data` — kind-specific data dict (the `init_data()` accumulator, see [Results dict shape](#results-dict-shape)).
- `subject` — the task's target patterns / formatter target / etc. `setup_phase` sets it on the first emit so handlers can read it during init; a later emit may carry a refined subject (handlers latch the last non-empty value and refresh the surface title).
- `final` — `True` on the terminal emit.

There is **no** separate `task_started` / `task_complete` hook — the first and last `task_update` carry that intent. The `subject` rides on the first update so init has what it needs without a dedicated start signal.

The order at runtime, end-to-end:

```
setup_phase(ctx, lifecycle, subject, …)          # FIRST thing in every _impl
  └─ lifecycle.task_update(🔧 Setup, subject=…)   # the first update
       └─ GithubStatusChecks inits → creates the check run
       └─ BuildkiteAnnotations inits → posts the first "info" annotation
  └─ hc_trait.health_check                        # Workflows env table / server health check
bazel_trait.build_start
  └─ Workflows prints `--- :bazel: Running bazel <task> [<task-name>] <targets>`
events = bazel.build_events.iterator()           # create handle BEFORE the spawn
ctx.bazel.build(..., build_events = [events])    # runtime subscribes pre-spawn
data["sink_invocation_id"] = build.sink_invocation_id
lifecycle.task_update(running)                   # link surfaces in the annotation
for event in events:
    bazel_trait.build_event(ctx, event)          # ArtifactUpload records testlog paths
    if process_event(data, event):
        lifecycle.task_update(running)           # streamed metadata + targets
build_status = build.wait()
bazel_trait.build_end(ctx, build_status.code)    # ArtifactUpload uploads
... task-specific work (run formatter, parse SARIF, etc.) ...
lifecycle.task_update(passed | failed, final=True)  # terminal — final body
return <TaskConclusion>
```

---

## Customizing repro & fix suggestions

`TaskLifecycleTrait` has a second slot, `repro_fix_suggestion`, that lets a user `config.axl` accept, reject, or modify the `aspect …` / `bazel …` repro and fix commands tasks emit at terminal-emit time. Common uses: rewrite `aspect …` to an internal wrapper command, strip flags the user wants kept private, suppress fix suggestions deemed unsafe in their CI environment.

Built-in tasks populate `data["repro_commands"]` / `data["fix_commands"]` with `ReproFixCommand` records (defined in [`private/lib/lifecycle.axl`](private/lib/lifecycle.axl)). The framework runs every registered handler over un-hooked entries on each surface emit via `dispatch_task_update`, so no downstream consumer — CLI printer, GHSC check-run body, BK annotation, GitHub PR-comment rollup — ever sees an un-hooked entry. The `_hooked` flag on each record guarantees the hook chain runs at most once per entry, so producers and surfaces can emit freely.

Hook signature:

```python
def my_hook(ctx: TaskContext, info: ReproFixInfo) -> ReproFixSuggestion:
    # info carries typed task metadata + per-suggestion fields:
    #   info.command, info.description, info.slug,
    #   info.task_name, info.command_kind, ...
    # See private/lib/lifecycle.axl.

    return REPRO_FIX_ACCEPT                                            # keep as-is
    return REPRO_FIX_REJECT                                            # drop
    return repro_fix_replace(command = ...)                            # rewrite command
    return repro_fix_replace(description = ...)                        # retag description
    return repro_fix_replace(command = ..., description = ...)         # rewrite both
```

On `repro_fix_replace`, omitting `command` or `description` keeps the prior value, so hooks only need to set the fields they actually want to change. Scope decisions by `info.slug` (stable, kebab-case identifier per producer) rather than parsing the command string — see the catalogue in `apply_repro_fix_hooks`'s docstring. Replace verdicts inherit the prior slug automatically; suggestion identity stays stable across rewrites.

Register from `config.axl`:

```python
load("@aspect//:traits.axl",
     "REPRO_FIX_ACCEPT",
     "REPRO_FIX_REJECT",
     "ReproFixInfo",
     "ReproFixSuggestion",
     "TaskLifecycleTrait",
     "repro_fix_replace")

# Suggestion slugs you want to drop globally.
_DROPPED = ("format-fix-vanilla-bazel", "gazelle-fix-vanilla-bazel")

def _reject_dropped_slugs(ctx: TaskContext, info: ReproFixInfo) -> ReproFixSuggestion:
    if info.slug in _DROPPED:
        return REPRO_FIX_REJECT
    return REPRO_FIX_ACCEPT

def _rewrite_aspect_for_lint(ctx: TaskContext, info: ReproFixInfo) -> ReproFixSuggestion:
    if info.task_name != "lint":
        return REPRO_FIX_ACCEPT
    return repro_fix_replace(
        command = info.command.replace("aspect ", "mywrapper "),
    )

def config(ctx: ConfigContext):
    lifecycle = ctx.traits[TaskLifecycleTrait]
    lifecycle.repro_fix_suggestion.append(_reject_dropped_slugs)
    lifecycle.repro_fix_suggestion.append(_rewrite_aspect_for_lint)
```

Multiple handlers chain in registration order — handler N sees handler N-1's post-verdict values on `info.command` / `info.description`. A `REPRO_FIX_REJECT` short-circuits the chain for that entry.

`ReproFixInfo` (defined in [`private/lib/lifecycle.axl`](private/lib/lifecycle.axl)) carries typed common-case fields — `task_path`, `task_name`, `task_group`, `kind`, `command_kind`, `command`, `description`, `slug`, `status`, `exit_code`, `bazel_subcommand`, `targets`, `failed_targets` — plus an open-ended `extras` dict each task stamps with kind-specific keys, and a `data` escape hatch with the full task data dict. Prefer typed fields and `extras`; `data` is unstable across releases.

---

## Traits the framework exposes

> **Legacy violations.** Several entries below predate the [state management rules](#state-management-patterns-and-rules) and carry **data fields** (strings, dicts, lists) used as cross-feature communication channels. These are abuse-pattern fields scheduled for migration to the patterns in the next section. They're listed here because they exist in the current code; do NOT use them as a template for new traits. The "Status" column flags which fields are legitimate event/contract surfaces and which are pending migration.

| Trait                        | Defined in                                                                | Slots                                                                                                       | Status / Purpose |
|------------------------------|---------------------------------------------------------------------------|-------------------------------------------------------------------------------------------------------------|------------------|
| **`BazelTrait`**             | [bazel.axl](bazel.axl)                                                    | `build_start`, `build_event`, `build_end`, `build_retry`, `build_event_sinks`, `task_flags`, `flags`, `startup_flags`, `extra_flags`, `extra_startup_flags`, `execution_log_sinks` | ✅ Clean. Shape every Bazel invocation in the task: extra flags, BES sinks, per-event hooks, build-end cleanup. All fields are callables / hook lists / declarative config the task reads. |
| **`HealthCheckTrait`**       | [private/lib/health_check.axl](private/lib/health_check.axl)                              | `health_check`                                                                                              | ✅ Clean. Hook lists only. |
| **`TaskLifecycleTrait`**     | [private/lib/lifecycle.axl](private/lib/lifecycle.axl)                                    | `task_update`, `repro_fix_suggestion`                                                                       | ✅ Clean. Hook lists only. `task_update` drives status surfaces (first update inits, `final=True` concludes — see lifecycle section above). `repro_fix_suggestion` lets a user `config.axl` accept / reject / modify repro & fix command suggestions — see [Customizing repro & fix suggestions](#customizing-repro--fix-suggestions). |
| **`DeliveryTrait`**          | [delivery.axl](delivery.axl)                                              | `delivery_start`, `delivery_target`, `delivery_end`, `delivery_manifest`, `render_manifest_file`, `upload_manifest` | ✅ Clean. Hook callables only. |
| **`ArtifactsTrait`**         | [private/lib/artifacts.axl](private/lib/artifacts.axl)                                    | `artifacts_browse_url`, `artifact_urls`, `testlogs_label_urls`                                              | ⚠️ **Legacy** — data fields used as cross-feature state. Migrating to Pattern 2 (feature-owned + Callable trait). See [Artifact uploads](#artifact-uploads). |
| **`GitHubStatusChecksTrait`**| [feature/github_status_checks.axl](feature/github_status_checks.axl)      | `templates`, `metadata_keys`                                                                                | ⚠️ **Legacy** — user-facing config on a trait. Migrating to feature `args`. |
| **`GitHubCheckRunTrait`**    | [private/lib/checkrun.axl](private/lib/checkrun.axl)                                      | `html_url`                                                                                                  | ⚠️ **Legacy** — single-value cross-feature handoff via data field. Migrating to Pattern 2. |
| **`TipsTrait`**              | [private/lib/tips.axl](private/lib/tips.axl)                                              | `tips`, `silenced_tips`, `auto_print`                                                                       | ⚠️ **Legacy** — list/dict data fields as cross-feature state. Migrating to Pattern 2 (feature-owned tips with `tips.append`/`tips.collect` wrapper API). |
| **`GitHubApiRateLimitTrait`**| [private/lib/rate_limit.axl](private/lib/rate_limit.axl)                                  | `observations`, `watermark`, `limits`, `last_effective_factor`, `sibling_observations`, …                   | ⚠️ **Legacy** — many-to-many accumulator with cross-task swarm. Migrating to Pattern 3 (library-owned + tmpdir/JSON), because no single feature owns it and swarm aggregation crosses task boundaries. |
| **`LintTrait`**              | [lint.axl](lint.axl)                                                      | `lint_start`, `lint_report`, `lint_patch`, `lint_end` (✅ clean — hook lists) | Mixed. Hook lists are clean; the data fields below are legacy. |
| `LintTrait.changed_files`    |                                                                           |                                                                                                              | ⚠️ Legacy — task→features data on a trait field. Migrate to hook signature: `lint_start(ctx, changed_files)`. |
| `LintTrait.suggestions`, `LintTrait.comment_urls` |                                                      |                                                                                                              | ⚠️ Legacy — feature→task data on a trait field. Migrate to reversed hook return value or a Pattern-2 wrapper. |
| `LintTrait.findings_destination` |                                                                       |                                                                                                              | ⚠️ Legacy — user-facing config. Migrate to feature `args`. |

> **Note:** All public traits are re-exported from [`traits.axl`](traits.axl) for user `config.axl` ergonomics (`load("@aspect//:traits.axl", ...)`). Internal modules load from the owning module directly — the facade is a user-facing convenience, not the source of truth.

Tasks declare which traits they *use* in the `traits = [...]` arg of the `task(...)` call:

```python
build = task(
    name = "build",
    traits = [BazelTrait, HealthCheckTrait, TaskLifecycleTrait,
              GitHubStatusChecksTrait, ArtifactsTrait],
    implementation = _impl,
    ...
)
```

Features declare which traits they *register on* implicitly by referencing them in the implementation closure:

```python
def _artifact_upload_impl(ctx: FeatureContext):
    bazel_trait    = ctx.traits[BazelTrait]
    artifacts_trait = ctx.traits[ArtifactsTrait]
    ...
    bazel_trait.build_event.append(_on_build_event)   # subscribe
    bazel_trait.build_end.append(_on_build_end)       # subscribe
```

---

## State management: patterns and rules

This is the **load-bearing section for contributors writing new features**. Get it wrong and you'll either reintroduce the trait-abuse pattern (see the ⚠️ rows in the table above) or fight the framework. The three patterns below cover every legitimate case; if you think your situation needs something else, push back on the situation, not the pattern set.

### Three patterns, choose one

| Pattern | When to use | Where state lives | API shape |
|---|---|---|---|
| **1. Per-feature closure** | State is private to one feature (no other feature reads it) | Local variables in `_impl(ctx)`, closed over by the hooks the feature registers | Internal — never exposed outside the feature |
| **2. Feature-owned + Callable trait** | State has a single natural owner feature; other code reads/writes via that feature's public API; within-task only | Owner feature's `_impl(ctx)` closure | Wrapper struct (`tips.append`, `artifacts.append`, …) backed by `Callable` fields on a private trait |
| **3. Library-owned + tmpdir/JSON** | No single owning feature (many participants), or cross-task aggregation needed | `<tmpdir>/<libname>/state.json` via `record` + `json.encode/decode` | Wrapper struct backed by library functions that load/mutate/save the file |

Choose by ownership and scope:

- One feature is the natural owner of the state? → **Pattern 2.**
- No single owner / every caller is a peer participant, or cross-task aggregation needed? → **Pattern 3.**
- State is purely internal to one feature and no other feature ever reads it? → **Pattern 1.**

### Pattern 1: Per-feature closure (the default)

If your feature has state that's purely private — a counter, a debouncer, a buffer no other feature reads — keep it in `_impl(ctx)`:

```python
def _impl(ctx):
    seen_targets = {}     # private to this feature
    last_emit_ms = [0]    # boxed so closures can mutate

    def _on_event(ctx, event):
        if event.label in seen_targets:
            return
        seen_targets[event.label] = True
        ...

    ctx.traits[BazelTrait].build_event.append(_on_event)

my_feature = feature(_impl)
```

Each task gets its own `_impl` invocation, so each task has its own closure-scoped state. Nothing on a trait, nothing on `ctx`, nothing on disk. The state exists for exactly the task's lifetime.

Most existing features already use this pattern for their private state — there's nothing new here. It's listed first so the contrast with Patterns 2 and 3 is clear.

### Pattern 2: Feature-owned + Callable trait

When state has a single natural owner feature *and* other features (or the task itself) need to read or write to it. Canonical examples: tips, check-run URL, artifact registry.

The owner feature holds the state in its closure. Its public API is exposed as `Callable` fields on a trait. A library file declares the trait + wrapper struct; consumers use the wrapper. The owner feature is **co-located with the trait** in the same `private/lib/<name>.axl` file so the trait stays module-private.

```python
# private/lib/tips.axl — public API, owner feature, trait declaration, all co-located.

_Tip = record(
    id = field(str),
    severity = field(str),
    body = field(str, default = ""),
)

# Trait — implementation detail. Underscore-prefixed; never exported by name.
# Defaults are null-objects: silent no-op for void returns, empty for value returns.
_TipTrait = trait(
    append  = field(typing.Callable[[_Tip], None],   default = lambda t: None),
    collect = field(typing.Callable[[], list[_Tip]], default = lambda: []),
)

# Wrapper struct — consumer-facing API. Hides the trait machinery.
tips = struct(
    Tip     = _Tip,
    append  = lambda ctx, tip: ctx.traits[_TipTrait].append(tip),
    collect = lambda ctx: ctx.traits[_TipTrait].collect(),
    TRAITS  = [_TipTrait],
)

# Owner feature — co-located so it can bind closures to the module-private trait.
def _impl(ctx):
    state = []

    def _append(tip):
        for existing in state:
            if existing.id == tip.id:
                return  # dedup-by-id
        state.append(tip)

    def _collect():
        return _sorted(state)

    ctx.traits[_TipTrait].append  = _append
    ctx.traits[_TipTrait].collect = _collect

tips_feature = feature(_impl)
```

Consumers anywhere — they never touch the trait, only the wrapper struct:

```python
load("./private/lib/tips.axl", "tips")

tips.append(ctx, tips.Tip(id = "...", severity = "suggestion", body = "..."))
for t in tips.collect(ctx):
    ...
```

Tasks register **only the trait surface** — never the owner feature:

```python
# build.axl, lint.axl, etc.
load("./private/lib/tips.axl", "tips")
load("./private/lib/artifacts.axl", "artifacts")

def task():
    return Task(
        traits = [BazelTrait, TaskLifecycleTrait, ...] + tips.TRAITS + artifacts.TRAITS,
        # NO `features = [...]` — see "Rules: what tasks must not do" below.
        ...
    )
```

**Why this works:**

- **State is real-private.** The closure scope is language-enforced. Other features can't reach `state` directly — they go through `tips.append`/`tips.collect`, which dispatch through the trait callables. The wrapper hides everything.
- **Null-object defaults give graceful degradation for free.** When the owner feature isn't loaded (user opted out, no CI host, feature disabled), the trait callables stay at their defaults. `tips.append(ctx, t)` becomes a silent no-op; `tips.collect(ctx)` returns `[]`. Consumers don't crash, don't need `if feature_loaded:` checks, don't have error paths to maintain. The disabled state propagates through pure data flow.
- **`TRAITS = [...]` is the only thing tasks see.** The underscore-prefixed `_TipTrait` never appears in any task or feature module. If the library later changes its implementation and stops needing trait registration (e.g., switches to Pattern 3), it sets `TRAITS = []` and no task module needs to change.
- **Tasks don't reference owner features.** Feature loading happens via a separate channel (user `.aspect/config.axl`, framework defaults, opt-in flags). The decoupling is what makes the null-object property useful end-to-end.

**Co-location is non-negotiable.** Splitting the owner feature into a separate `feature/<name>.axl` file forces you to re-export `_TipTrait` so the separate file can bind callbacks — which leaks the trait name out into the global namespace and defeats the encapsulation. Keep the trait, the wrapper struct, AND the owner feature in `private/lib/<name>.axl`. Other code only sees the exported `<name>` struct and `<name>_feature` symbol.

### Pattern 3: Library-owned + tmpdir/JSON

When state has no single natural owner (every caller is a peer participant) or cross-task aggregation is needed. Canonical example: rate-limit observations — every HTTP-making feature contributes, no one "owns" the rate-limit budget, and the swarm aggregator reads sibling tasks' observations.

The library declares a typed `record` for its state, serializes via `json.encode`/`json.decode`, reads/writes to a path under tmpdir that's hardcoded in the library and never appears outside it:

```python
# private/lib/rate_limit.axl — `json` is an AXL global; no load needed.

_State = record(
    observations = field(list, default = []),
    watermark = field(dict, default = {"app": None, "env": None}),
    # ...
)

# Library-owned path. Hardcoded here; never appears in any task or feature module.
_FILE = "rate_limit/state.json"

def _tmpdir(ctx):
    return ctx.std.env.var("ASPECT_WORKFLOWS_RUNNER_JOB_TMPDIR") or ctx.std.env.temp_dir()

def _load(ctx):
    path = _tmpdir(ctx) + "/" + _FILE
    if not ctx.std.fs.exists(path):
        return _State()
    return _State(**json.decode(ctx.std.fs.read(path)))

def _save(ctx, state):
    path = _tmpdir(ctx) + "/" + _FILE
    ctx.std.fs.create_dir_all(path.rsplit("/", 1)[0])
    # AXL evaluator is single-threaded per task — no races between hooks
    # within one task. If load-modify-save becomes a profile hotspot, the
    # natural next move is an in-memory cache backed by rusqlite/Turso
    # keyed by task identity. For now, plain JSON load-modify-save is the
    # cheap, correct answer.
    ctx.std.fs.write(path, json.encode(state))

def _record(ctx, bucket, headers):
    s = _load(ctx)
    s.observations.append({"bucket": bucket, "ts": ..., "remaining": ..., ...})
    _save(ctx, s)

# Public API — wrapper functions take ctx, hide tmpdir/json/_State.
rate_limit = struct(
    record          = _record,
    should_throttle = lambda ctx, bucket, **kw: _should_throttle(ctx, bucket, **kw),
    usage_footer    = lambda ctx, host: _usage_footer(ctx, host),
    TRAITS          = [],   # no trait registration needed for Pattern 3
)
```

**Why this works:**

- **No trait registration.** Tasks don't list anything in their trait list (`TRAITS = []`); the encapsulation is purely the library-owned path. Underscore-prefixed `_FILE`, `_load`, `_save`, `_State` are module-private.
- **Cross-task IPC falls out for free.** Sibling readers open sibling task tmpdirs by path convention (each task writes to its own tmpdir; aggregators read multiple tmpdirs). No locking — single-writer per file.
- **Single-threaded AXL evaluator** → no concurrent writes within a task, no races, no locking primitives needed.
- **The `record` type drives ser/de.** No hand-rolled format, no parse-tolerance code. `json.encode(state)` and `_State(**json.decode(...))` round-trip cleanly.

### Rules: what features must NOT do

1. **No data fields on traits.** Trait fields are exclusively contract surfaces: `Callable` (single function — event handler, method, transform, predicate), `list[Callable]` (hook lists), or declarative config the task reads at dispatch time. **A `str`, `dict`, `list`, or any non-callable data type on a trait is the abuse pattern** and is forbidden in new code. The ⚠️ rows in the trait table above are legacy violations being migrated out. If you need to share data across features, use Pattern 2 or 3, never a data field on a trait.

2. **No reaching into other features' state.** Features are anonymous to each other. Feature A never imports feature B and calls into its closures, accesses its variables, or reads its private state. Cross-feature data flows ONLY through:
   - **Hook signatures** — events the task fires through trait hook lists (see [The per-task lifecycle](#the-per-task-lifecycle))
   - **Pattern-2 wrappers** — `tips.append(ctx, ...)`, `artifacts.append(ctx, ...)`, etc.
   - **Pattern-3 wrappers** — `rate_limit.record(ctx, ...)`, etc.

3. **No new shared-state primitives.** Don't add a Bag, an accumulator type, a `ctx.std.kv`, a `ctx.std.db`, a per-task slot for arbitrary feature data, or any other ambient surface that lets two features pass data to each other without going through the patterns above. Every such proposal in framework history has been rejected because it relocates the abuse pattern under a different name. The three patterns above are sufficient.

4. **No user-facing configuration on traits.** Knobs the user sets in `.aspect/config.axl` (templates, formatters, severity overrides, behavior flags) belong on the feature's `args` surface, not as trait fields. A trait field that exists for users to write to in their config is the same abuse pattern as data sharing between features — `GitHubStatusChecksTrait.templates`, `LintTrait.findings_destination`, and `TipsTrait.silenced_tips` are legacy violations of this rule.

5. **Don't bypass the wrappers.** Even when a trait or library is reachable, callers should always go through `lib.method(ctx, ...)`. Writing `ctx.traits[X].method(...)` directly couples the caller to the trait shape, prevents the library from changing its implementation, and defeats the wrapper's encapsulation. **The trait is implementation; the struct is API.**

6. **Don't reach for in-memory shared-state shortcuts that AXL appears to support.** Module-level mutable state, "process-global" caches, ambient lookup mechanisms — even when they'd "work," they violate the patterns above and create state that survives task boundaries in ways that aren't auditable. If you find yourself wanting one, you're solving a Pattern-3 problem (cross-task aggregation) and should use tmpdir/JSON.

### Rules: what tasks must NOT do

1. **Tasks must not reference features.** A task module (`build.axl`, `test.axl`, `lint.axl`, `format.axl`, `gazelle.axl`, `delivery.axl`) declares its trait surface and its `_impl(ctx)`. It never imports a feature name, never lists features in its `Task(...)` constructor, never knows which features will attach to its traits at runtime. **Feature loading is a separate channel** — framework default, user `.aspect/config.axl` opt-in, etc. The decoupling is what makes graceful degradation work: a task that lists `+ artifacts.TRAITS` works identically whether or not the artifacts feature is loaded, because the trait callable defaults are null-objects.

2. **Tasks must iterate every trait hook list they declare.** Declaring `BazelTrait` and then never calling its `build_event` or `build_end` hooks means features that subscribed to those slots silently don't fire — and the failure mode is "things mysteriously don't work," not a loud error. The [canonical anatomy](#anatomy-of-a-task-_impl) shows the full iteration order; follow it.

3. **Tasks must not touch other tasks' state.** Each task instance is hermetic — its closures, its trait instance, its tmpdir are scoped to it alone. If you find yourself wanting one task to read another's state, that's [Pattern 3](#pattern-3-library-owned--tmpdirjson) (cross-task aggregation via sibling-tmpdir reads), not a direct task-to-task communication channel.

### Cheatsheet

When designing new state-bearing code, walk this list top to bottom and stop at the first match:

1. Is the state purely private to one feature? → **Pattern 1** (closure).
2. Does one feature naturally own it and others read/write? → **Pattern 2** (Callable trait + wrapper). Co-locate trait + owner feature in `private/lib/<name>.axl`.
3. Many anonymous peer participants, or cross-task aggregation needed? → **Pattern 3** (record + JSON in tmpdir).
4. None of the above fit? → You've probably misframed the problem. Most "new" cases are actually misframed events — the data should flow as a hook argument or a hook return value through a trait the task or producing feature already exposes. Re-read [The per-task lifecycle](#the-per-task-lifecycle) before reaching for a fourth pattern.

---

## BES streaming: handle-based iterator API

BES events reach AXL through a broadcaster (see [`crates/axl-runtime/src/engine/bazel/stream/broadcaster.rs`](../../../../axl-runtime/src/engine/bazel/stream/broadcaster.rs)). It's fire-and-forget: every `send` clones into every subscriber's mpsc channel and never blocks. The broadcaster has no opinion about back-pressure or replay — subscribers manage their own buffering.

Tasks that need to consume events do so via an explicit iterator handle:

```python
events = bazel.build_events.iterator()
build = ctx.bazel.build(..., build_events = [events])
for event in events:
    ...
```

The handle is created *before* `ctx.bazel.build(...)` and passed in via `build_events=[...]`. The runtime subscribes the receiver inside `Build::spawn`, before bazel opens the BEP FIFO — so the early burst (`build_started`, `target_completed`, `named_set_of_files`) is buffered for the consumer regardless of when iteration actually starts. The race window present with the old lazy `build.build_events()` is closed by construction: you can't pass a handle that doesn't exist yet, and once passed it's already subscribed.

Handles are single-use; reusing one in a second `ctx.bazel.build(...)` call errors. For retries, create a fresh handle per attempt.

Optional `kinds=` filter narrows the stream at iteration time:

```python
events = bazel.build_events.iterator(
    kinds = [build_event.TargetCompleted, "named_set_of_files"],
)
```

The mpsc channel between the broadcaster and the iterator is unbounded; iterate promptly to keep memory in check, or call `events.drain()` to stop accumulating events.

---

## Per-kind result libraries

Every task that surfaces data goes through a `private/lib/<kind>_results.axl` library that owns the rendering for that task type. The contract:

```python
def init_data():
    """Return a fresh data dict. Should derive from bazel_results.init_data()
    so process_event() can populate the full bazel state."""

def render_check_output(ctx, data, status, render_ctx, links,
                        templates = None, metadata_keys = None):
    """Return {title: str, summary: str, text: str} suitable for both a GitHub
    check-run output and a Buildkite annotation body."""
```

The lint / format / gazelle / delivery libraries all start their `init_data()` from `bazel_results.init_data()` and namespace their kind-specific extensions under a sub-dict keyed by the kind name:

```python
load("./bazel_results.axl", bazel_init_data = "init_data")

def init_data():
    r = bazel_init_data()
    r["lint"] = {
        "diagnostics":    [],
        # ... other lint-specific keys ...
    }
    return r
```

…and append `SHARED_DETAILS_BODY_TEMPLATE` (exported from `bazel_results.axl`) to their `_DETAILS_TEMPLATE`:

```python
load("./bazel_results.axl", "SHARED_DETAILS_BODY_TEMPLATE", "build_details_data")

_DETAILS_TEMPLATE = """{% if no_files_to_format %}
... task-specific top section ...
{% endif %}
""" + SHARED_DETAILS_BODY_TEMPLATE
```

Result: every task — even gazelle and format — renders the same `### Bazel targets` / `### Bazel details` body sections at the bottom. Adding a sub-block under `### Bazel details` in `bazel_results` automatically appears across every task type with no per-kind change.

### Results dict shape

The dict returned by `init_data()` is a two-tier structure: **task-managed scalars at the root**, **BES-derived bazel state under `["bazel"]`**. The split mirrors the rendered output — everything below `### Bazel details` reads from `data["bazel"][...]`, while task-driven scalars (start/finish time, target pattern, sink invocation id, reproducer command) stay at the root for code that doesn't run bazel.

```
data = {
  ┌─ Task-managed (root) ──────────────────────────────────────────────┐
  │ "start_time_ms":             0,    # task wall start (epoch ms)    │
  │ "finish_time_ms":            0,    # task wall end (epoch ms)      │
  │ "target_pattern":            "",   # joined args.targets           │
  │ "sink_invocation_id":        "",   # gRPC BES sink UUID (final)    │
  │ "repro_commands":            [],   # producer-authored entries:   │
  │ "fix_commands":              [],   #   {"command", "description"} │
  │ "bazel_attempts":            [],   # per-attempt history when      │
  │                                    # bazel retried (else empty);   │
  │                                    # each entry has {attempt,      │
  │                                    # exit_code, sink_invocation_id,│
  │                                    # bazel: <full bazel sub-dict>} │
  └────────────────────────────────────────────────────────────────────┘

  ┌─ Bazel state (BES-derived) ────────────────────────────────────────┐
  │ "bazel": {                                                         │
  │   # Failures                                                       │
  │   "failed_targets":          [...],   "failed_actions":      [...] │
  │   "failed_actions_total":    0,       "failed_action_labels": [...]│
  │                                                                    │
  │   # Tests                                                          │
  │   "failed_tests":            [...],   "flaky_tests":          [...]│
  │   "passed_tests":            [...],   "test_details":          {}  │
  │   "passed_test_total":       0,       "failed_test_total":      0  │
  │   "flaky_test_total":        0,       "cached_test_total":      0  │
  │   "executed_test_total":     0,                                    │
  │   "executed_duration_ms":    0,       "cached_duration_ms":     0  │
  │                                                                    │
  │   # Targets                                                        │
  │   "target_kinds":            {},      "built_targets":        [...]│
  │   "targets_total":           0,                                    │
  │   "targets_configured":      0,                                    │
  │   "targets_configured_not_including_aspects": 0,                   │
  │   "targets_configured_counted":               0,                   │
  │                                                                    │
  │   # Action metrics                                                 │
  │   "actions_executed":        0,       "actions_cached":         0  │
  │   "actions_total":           0,       "action_mnemonics":       {} │
  │   "runner_counts":           {},      "action_cache_stats":     {} │
  │                                                                    │
  │   # Bazel timing (BES build_metrics; distinct from task wall)      │
  │   "wall_time_ms":            0,       "cpu_time_ms":            0  │
  │   "analysis_phase_time_ms":  0,       "execution_phase_time_ms":0  │
  │   "critical_path_ms":        0,                                    │
  │                                                                    │
  │   # Build setup                                                    │
  │   "packages_loaded":         0,                                    │
  │   "num_analyses":            0,       "num_builds":             0  │
  │                                                                    │
  │   # Invocation identity                                            │
  │   "invocation_id":           "",      "bes_results_url":       ""  │
  │   "build_tool_version":      "",      "build_command":         ""  │
  │   "cpu_arch":                "",      "compilation_mode":      ""  │
  │   "workspace_directory":     "",      "working_directory":     ""  │
  │   "server_pid":              0,                                    │
  │                                                                    │
  │   # User-facing key-value tables                                   │
  │   "metadata":                {},   # build_metadata BES event      │
  │   "workspace_status":        {},   # workspace_status BES event    │
  │   "options_parsed":          {                                     │
  │     "explicit_cmd_line":     [],   # for the displayed command     │
  │     "startup_options":       [],                                   │
  │     "cmd_line":              [],                                   │
  │   },                                                               │
  │                                                                    │
  │   # Termination                                                    │
  │   "aborted":                 False,                                │
  │   "abort_reason":            "",      "abort_description":     "" │
  │ },                                                                 │
  └────────────────────────────────────────────────────────────────────┘

  ┌─ Per-kind extensions (added by private/lib/<kind>_results.init_data) ───┐
  │ # lint     → data["lint"]                                          │
  │   "diagnostics", "strategy", "build_failed", "linter_exit_code",   │
  │   "changed_files", "tools_run",                                    │
  │   "counts_by_severity", "counts_by_tool"                           │
  │ # format   → data["format"]                                        │
  │   "scope", "formatter_target", "severity_resolved",                │
  │   "affected_files", ...                                            │
  │ # gazelle  → data["gazelle"]                                       │
  │   "gazelle_target", "check_only", "severity_resolved",             │
  │   "dirs", "affected_files", ...                                    │
  │ # delivery → data["delivery"]                                      │
  │   "prefix", "commit_sha", "build_url", "ci_host", "mode", ...      │
  └────────────────────────────────────────────────────────────────────┘
}
```

**Why the split:**

- *Bazel-derived* state only exists when bazel actually ran. Non-bazel tasks (or pre-bazel phases) leave `data["bazel"]` at its zero defaults, and the renderer skips empty sub-blocks.
- *Task-managed* scalars are populated by AXL code (e.g. `now_ms(ctx)` for `start_time_ms`), independent of any bazel invocation. They're read by code that doesn't care about bazel — the lifecycle / status surface plumbing.
- *Per-kind extensions* are namespaced under their kind sub-dict (`data["lint"][...]`, `data["format"][...]`, etc.) so the schema is self-documenting: a reader sees `data["gazelle"]["affected_files"]` and immediately knows it's gazelle's notion of affected files (vs. format's, which lives at `data["format"]["affected_files"]` — different semantics, same key).

**Reading conventions:**

- `process_event(data, event)` writes BES events into `data["bazel"][...]`. Task code that drives bazel (`bazel_runner.run_bazel_task`, `lint._impl`, etc.) calls `process_event` for each BES event.
- Renderers (`build_summary_data`, `build_details_data`, `build_invocation_rows`, `_build_invocation_stats`, …) read primarily from `data["bazel"][...]`. The `<task>_results.axl` libraries delegate to those for the shared sections and add their own per-kind data on top.
- Status surface handlers (`feature/buildkite_annotations.axl`, `feature/github_status_checks.axl`, `feature/github_status_comments.axl`) receive the dict via `update.data` and pass it through to renderers — they don't read individual keys themselves except for severity/conclusion classification, which uses a small, well-known set of `bazel.*` counters.

**Mirrors the rendered structure:**

The Aspect Web UI exposes three tabs per invocation: *Targets / Logs / Details*. The status surface body mirrors that shape:

| UI tab | Status surface section | Data source |
|---|---|---|
| Targets | `### Bazel targets` | `data["bazel"]["passed_tests"]`, `failed_tests`, `built_targets`, `failed_actions`, … |
| (Logs) | (not on status surfaces — the CI log link covers it) | — |
| Details | `### Bazel details` (umbrella with `<details>` sub-blocks) | `data["bazel"]["options_parsed"]`, `metadata`, `workspace_status`, `action_mnemonics`, … |

Plus task-level sections at the top (primary task data, Artifacts, Task timing) and a separate `### Runner metadata` section for Aspect Workflows runner identity (`data["bazel"]` is the *invocation* it ran; the runner metadata is the *machine* — kept distinct).

---

## Status checks and annotations

[`private/lib/check_dispatch.axl`](private/lib/check_dispatch.axl) holds the parts both `GithubStatusChecks` and `BuildkiteAnnotations` would otherwise duplicate:

```python
load("./private/lib/check_dispatch.axl",
    "RENDERERS",          # kind → struct(init, render)
    "renderer_for_kind",
    "kind_for_task",      # task_name → kind for the initial pre-update render
    "to_display_name",
    "make_render_ctx",    # build the render_ctx dict from feature state
    "apply_artifact_links",  # copy ArtifactsTrait fields onto links
    "should_emit_update",    # throttle non-final updates, always pass terminals
)
```

When a new task kind is added (see [below](#how-to-add-a-new-task-kind-new-renderer)), the only change to the dispatch is a single entry in `private/lib/check_dispatch.axl`'s `RENDERERS` table. Both features pick it up automatically.

### Summary header rows

The `· `-separated rows above the **Last update** line (the same data on GH check-runs, BK annotations, and GHSC summary comments) are built by `bazel_results.build_invocation_rows`. They are grouped semantically — each row answers one question, kept short enough that the row fits on a single visual line in narrow surfaces.

| Row | Question it answers      | Items                                                                                                           |
|-----|--------------------------|-----------------------------------------------------------------------------------------------------------------|
| 1   | **What & when?**         | status icon + label · `📅` ISO date · `⏱` elapsed with phase breakdown                                          |
| 2   | **Who & what tools?**    | `👤` user · `🖥` build host · `💎` aspect-cli version                                                           |
| 3   | **Where did the code come from?** | `🐙`/`🦊`/`🦝` repo · `🔀` PR · `⎇` branch · `✏️` commit · `🤖` trigger                                  |
| 4   | **What did bazel do?**   | `📦` targets · `⚡` actions executed/cached · `🧪` tests run/cached · `🔧` bazel cmd · `🎯` CPU · `⚙️` config   |
| 5   | **Where did it run?**    | `🚀` Workflows runner version · `☁️` cloud · `📍` AZ · `💠` instance type · `🆔` instance ID                    |
| 6   | **Project taxonomy**     | `🔗` custom links · `🏷` tags                                                                                    |
| 7   | **Where to dig deeper?** | `✨` Aspect Workflows record · `🐙`/`🐝` CI host (Actions / Buildkite / GitLab / Circle)                        |

Why the split is shaped this way:

- **Row 1 stays minimal.** Rendered on a narrow CR widget (BK left rail, GH check-run header) the first row should show outcome + duration with no horizontal pressure. The phase breakdown after `⏱` already adds 4–8 short tokens, so adding user/host/CLI on top wraps into multiple visual lines on most surfaces.
- **Row 2 (actor & toolchain) is grouped because the items are about *the run's identity* — who started it, on what machine, with which CLI build.** Together they're the "blame & repro" axis: triaging a bad run usually wants all three at once.
- **Rows 3 (VCS) and 4 (build content) form the natural pair: where the code came from + what bazel did with it.** A reader who clicks into a run for "what was the load like?" reads row 4; "what's the change under test?" reads row 3.
- **Row 5 (Workflows runner) is gated on actually running on an Aspect Workflows runner.** Off-Workflows runs (a developer laptop, a generic CI host) skip it entirely instead of rendering five empty cells.
- **Row 6 (tags + custom links) holds project-specific taxonomy.** Workflows configs add custom links and tags that don't have a fixed semantic meaning to the renderer; collecting them into one row keeps the structured rows above unpolluted.
- **Row 7 (external links) is "click here for more."** The Aspect Web UI link and the CI host link are entry points to surfaces with deeper detail than the check-run body — keeping them at the bottom matches the read order: scan the structured data above, then jump.

Empty rows are skipped — the per-row `if row:` filter in the assembly loop keeps the rendered output tight when a row has no items (e.g. a non-Workflows run skips row 5 cleanly).

Feature-local responsibilities (which can't be lifted because the surfaces differ):

- **`GithubStatusChecks`** authenticates against GitHub via the Aspect Workflows GitHub App, fetches the running job URL via the Actions REST API, posts annotations only on the *terminal* update (GitHub appends across PATCHes — running emits would duplicate findings), and supports lint annotations + multi-batch chunking.
- **`BuildkiteAnnotations`** probes for `buildkite-agent` on PATH, builds a leading task pill (`[:aspect: task <key>]`) so a step that runs multiple tasks gets distinguishable annotations, and maps `(kind, data, status)` to `--style success/info/warning/error` via `_severity_for`.

---

## Artifact uploads

`feature/artifacts.axl` registers two callbacks on the Bazel trait:

- `_on_build_event(ctx, event)` on `bazel_trait.build_event` — records testlog paths, profile output, etc. as the BES stream goes by.
- `_on_build_end(ctx, exit_code)` on `bazel_trait.build_end` — uploads the recorded files via the host CI's artifact API (GitHub Actions REST, `buildkite-agent artifact upload`, CircleCI `store_artifacts`, GitLab Generic Packages), then publishes the resulting URLs.

For these to fire, the task **must** iterate `bazel_trait.build_event` inside its BES loop and `bazel_trait.build_end` after `build.wait()`. Skip either and no upload runs. The canonical pattern is in the [Anatomy of a task](#anatomy-of-a-task-_impl) section below.

### URL publication API

The artifact uploader publishes URLs via the [Pattern 2](#pattern-2-feature-owned--callable-trait) wrapper API:

```python
load("./private/lib/artifacts.axl", "artifacts")

# Producers (the upload feature, format's diff-upload hook, gazelle's diff-upload hook)
# register their artifacts as they're uploaded:
artifacts.append(ctx, artifacts.Artifact(
    kind = "testlogs",
    url  = direct_download_url,
    name = "testlogs.tar.gz",
))

# Per-bazel-target test files use the `label` field:
artifacts.append(ctx, artifacts.Artifact(
    kind  = "testlog_file",
    url   = url,
    name  = filename,           # "test.log", "test.xml", ...
    label = "//foo:bar_test",
))

# format / gazelle hooks:
artifacts.append(ctx, artifacts.Artifact(
    kind = "format_diff",
    url  = uploaded_url,
    name = "format.patch",
))

# Consumers (check renderer, format/gazelle for their own data) query the derived views:
artifacts.list(ctx)                          # → list[Artifact]
artifacts.by_kind(ctx)                       # → {kind: url}, non-label entries
artifacts.by_label(ctx)                      # → {label: {filename: url}}, per-target entries
artifacts.browse_url(ctx)                    # → env-derived CI artifacts page URL (no setter — derived live)
```

`artifacts.TRAITS` is the registration token the task splats into its trait list (`traits = [...] + artifacts.TRAITS`). The owner feature is `artifacts_upload`, co-located in `private/lib/artifacts.axl`. When the upload feature isn't loaded, the trait defaults make every `artifacts.append(...)` a silent no-op and every read return empty — the format/gazelle hooks call `append` unconditionally and Just Work whether or not the feature is active.

`artifacts.browse_url(ctx)` is **not** a stored value — it's a thin re-export of `detect_artifacts_browse_url(ctx)` from `private/lib/ci.axl`, which reconstructs the URL from CI host env vars (the GH Actions run page, the BK build's canvas tab, etc.) on every call. The render-time decision "should I surface this link?" belongs to the consumer: typically gated by `if artifacts.list(ctx):`.

> **Current implementation note.** As of this writing, the codebase still uses the legacy `ArtifactsTrait` with `artifacts_browse_url` / `artifact_urls` / `testlogs_label_urls` data fields. The migration to the wrapper API above is in progress. New code should be written against the wrapper; legacy callsites in `feature/github_status_checks.axl`, `feature/buildkite_annotations.axl`, `private/lib/bazel_results.axl`, `format.axl`, and `gazelle.axl` are scheduled to be ported. Do not extend the legacy fields.

The check-run / annotation features consume artifact URLs via the wrapper's derived views (`by_kind`, `by_label`) routed through `apply_artifact_links` in `private/lib/check_dispatch.axl`. The renderers suppress the Artifacts row when both views are empty, so a task that didn't upload anything stays clean.

---

## Anatomy of a task `_impl`

The canonical flow, with annotations. Use it as a checklist when reading or writing a task:

```python
def _impl(ctx: TaskContext) -> int | TaskConclusion:
    bazel_trait = ctx.traits[BazelTrait]
    hc_trait    = ctx.traits[HealthCheckTrait]
    lifecycle   = ctx.traits[TaskLifecycleTrait]

    data = init_data()
    data["start_time_ms"]  = now_ms(ctx)
    data["target_pattern"] = " ".join(ctx.args.targets)

    # 1. The single pre-task setup phase — FIRST thing in every _impl. It emits
    #    the `🔧 Setup` phase mark (the task's first task_update): inits every
    #    status surface AND renders their first body from `kind` + `data`.
    #    `subject` rides along for the surface title. It also resolves Bazel
    #    flags + parses the workspace .bazelrc and runs the health_check hooks —
    #    a failed health check concludes the surface and fail()s the task inside
    #    setup_phase (it does not return). Returns the resolved command flags
    #    (None for a non-Bazel task). See `setup_phase` in private/lib/lifecycle.axl.
    flags = setup_phase(ctx, lifecycle, data["target_pattern"], "<task>_results", data, hc_trait, bazel_trait, "build")

    # 2. Pass build_event_sinks (gRPC sinks the runtime registers BEFORE
    #    `Build::spawn` returns) so BES events reach the Aspect backend and
    #    the "Aspect Workflows" link resolves to a real invocation. The iter
    #    handle subscribes pre-spawn — no late-subscribe race.
    events = bazel.build_events.iterator()
    build_events = [events] + list(bazel_trait.build_event_sinks)

    # 3. Fire build_start hooks BEFORE spawning Bazel. The `Workflows`
    #    feature registers the `--- :bazel: Running bazel <task> [<key>]
    #    <targets>` BK section marker here.
    for hook in bazel_trait.build_start:
        hook(ctx)

    build = ctx.bazel.build(flags = flags, build_events = build_events, *ctx.args.targets)

    # 4. Capture sink_invocation_id (the runtime mints it inside Build::spawn
    #    before returning) and emit a running update so the "Aspect Workflows"
    #    link surfaces in the live annotation as soon as the build is spawned.
    data["sink_invocation_id"] = build.sink_invocation_id
    task_update(ctx, lifecycle, "running", "Building...", kind = "<task>_results", data = data,
                phase = Phase(name = "build", description = "Build targets", emoji = "🔨"))

    # 5. Drain the event iterator. Per event: bazel_trait.build_event hooks
    #    (ArtifactUpload records testlog paths) + process_event to populate the
    #    bazel state and stream metadata into the live annotation.
    for event in events:
        for handler in bazel_trait.build_event:
            handler(ctx, event)
        if process_event(data, event):
            task_update(ctx, lifecycle, "running", "Building...", kind = "<task>_results", data = data)

    # 6. Wait for bazel, then fire build_end hooks (ArtifactUpload uploads).
    build_status = build.wait()
    for hook in bazel_trait.build_end:
        hook(ctx, build_status.code)

    if not build_status.success:
        data["<kind>"]["build_failed"] = True   # kind-specific failure flag
        return task_update(ctx, lifecycle, "failed", "Build failed", kind = "<task>_results",
                           data = data, final = True, exit_code = 1, conclusion = conclusion("failed", data))

    # 7. ... task-specific post-build work (run binary, parse output, etc.)

    # 8. Terminal task_update (final=True returns a TaskConclusion)
    status = "passed" if exit_code == 0 else "failed"
    if not data.get("finish_time_ms"):
        data["finish_time_ms"] = now_ms(ctx)
    if data.get("start_time_ms") and not data["bazel"].get("wall_time_ms"):
        data["bazel"]["wall_time_ms"] = data["finish_time_ms"] - data["start_time_ms"]
    result = task_update(ctx, lifecycle, status, "Done", kind = "<task>_results",
                         data = data, final = True, exit_code = exit_code, conclusion = conclusion(status, data))
    return result
```

The `lint.axl` impl is the closest to this template; `format.axl` and `gazelle.axl` are similar but pass a single target instead of a list and build the entrypoint in step 10. `delivery.axl` does multiple Bazel invocations (one per phase) — the same pattern just runs once per phase.

---

## How to add a new task

1. Create `crates/aspect-cli/src/builtins/aspect/<task_name>.axl` and follow the [canonical flow](#anatomy-of-a-task-_impl).
2. **Declare the trait surface.** If your task drives Bazel, declare `BazelTrait`. If it surfaces data in CI, declare `TaskLifecycleTrait`. If it should print runner-env / health-check sections on Buildkite, declare `HealthCheckTrait`. For artifact uploads, splat `+ artifacts.TRAITS` (NOT `ArtifactsTrait` — that's the legacy data-field shape being migrated out). Same for tips: `+ tips.TRAITS`.
3. **Do NOT list features** in the task constructor. Tasks declare trait surfaces only; feature loading happens through a separate channel (framework default, user `.aspect/config.axl`). See [Rules: what tasks must not do](#rules-what-tasks-must-not-do).
4. Decide the `task_update.kind`. If your task's data shape matches an existing renderer (build/test → `bazel_results`, lint → `lint_results`, etc.), reuse it. Otherwise add a new kind — see [How to add a new task kind](#how-to-add-a-new-task-kind-new-renderer).
5. Wire the task in `MODULE.aspect`'s task registry.
6. Add a row to [`README.md`](README.md).

You get GHSC + BK annotations + artifact uploads for free as long as you (a) iterate the trait hooks listed in the canonical flow, (b) emit the right `TaskUpdate.kind`, and (c) capture `sink_invocation_id` after `ctx.bazel.build(...)`. When the feature owning a particular wrapper isn't loaded, calls through that wrapper become silent no-ops — your task code doesn't need to know.

### If your new task needs to publish or consume cross-feature data

Read [State management: patterns and rules](#state-management-patterns-and-rules) before designing the data flow. The short version:

- Private to your task body? Just a local variable in `_impl(ctx)` (Pattern 1).
- One feature naturally owns it and others consume? Pattern 2 (Callable trait + wrapper, co-located in `private/lib/<name>.axl`).
- Many anonymous peer producers or cross-task aggregation? Pattern 3 (record + JSON in tmpdir).
- Adding a data field to a trait? Stop — that's the forbidden abuse pattern.

---

## How to add a new task kind (new renderer)

If your task's data shape doesn't fit any existing kind:

1. Create `private/lib/<kind>_results.axl` exporting:
   ```python
   def init_data():
       r = bazel_init_data()
       r["<kind>"] = {...task-specific fields...}
       return r

   def render_check_output(ctx, data, status, render_ctx, links,
                           templates = None, metadata_keys = None):
       return {"title": ..., "summary": ..., "text": ...}
   ```
2. Append `bazel_results.SHARED_DETAILS_BODY_TEMPLATE` to your `_DETAILS_TEMPLATE` so the rendered body has the same Targets / Build Metrics / Invocation / etc. tail as every other task.
3. In `private/lib/check_dispatch.axl`, add an entry to `RENDERERS` and (if your task name should resolve to this kind for the *initial* pre-update render) to `_TASK_KIND_DEFAULTS`.
4. Add a snapshot test at `private/lib/<kind>_results_test.axl` mirroring the existing ones.
5. Register the snapshot test as a task in `.aspect/config.axl` so `aspect dev test-<kind>-template-snapshots` runs it.

Both `GithubStatusChecks` and `BuildkiteAnnotations` will dispatch to your new renderer automatically — no per-feature changes.

---

## Testing

Three layers, listed in increasing fidelity:

### Snapshot tests (per-kind result libraries)

Each `private/lib/<kind>_results_test.axl` enumerates representative scenarios (clean / partial / full failure / aborted / large-trim / etc.) and prints the rendered output. Run with:

```sh
./target/debug/aspect-cli dev test-template-snapshots          # bazel_results
./target/debug/aspect-cli dev test-lint-template-snapshots
./target/debug/aspect-cli dev test-format-template-snapshots
./target/debug/aspect-cli dev test-gazelle-template-snapshots
./target/debug/aspect-cli dev test-delivery-template-snapshots
./target/debug/aspect-cli dev test-bk-annotation-snapshots     # BK pill + body
./target/debug/aspect-cli dev test-pr-comment-snapshots        # github_status_comments aggregated PR comment
```

These run end-to-end through `render_check_output` — they exercise the actual templates, so any layout regression shows up as a visible diff in the printed scenario.

### `tests axl` — AXL unit tests

`./target/debug/aspect-cli tests axl` runs all `test_case(tc, ..., "label")` calls in `.aspect/axl.axl` (~720+ cases). Add cases there for new helpers in `private/lib/*.axl`.

### Live CI

Each task is exercised on every CI provider (GitHub Actions, Buildkite, CircleCI, GitLab) via the per-task pipeline definitions in [`.buildkite/pipeline.yaml`](../../../../../.buildkite/pipeline.yaml), [`.github/workflows/ci.yaml`](../../../../../.github/workflows/ci.yaml), [`.gitlab-ci.yml`](../../../../../.gitlab-ci.yml), and [`.circleci/config.yml`](../../../../../.circleci/config.yml). Each task runs twice per CI host: once with `ASPECT_DEBUG=1` (the debug variant) and once without — debug emits the `target: "axl.log"` trace lines that the per-task `_impl` printers go to, which is the fastest way to diagnose live failures.

When a task you change works in snapshots but fails live (artifact URL malformed, BK section missing, etc.), capture the `ASPECT_DEBUG=1` step's stdout from the BK / GHA UI — every helper logs enough state to pin down the failure mode without re-running locally.

---

## Quick checklist for new contributors

When writing a new task, feature, or library, sanity-check:

**Task lifecycle / BES streaming**

- [ ] If the task iterates BES events, create the iterator handle with `bazel.build_events.iterator()` *before* calling `ctx.bazel.build(...)`, and include it in the `build_events=[...]` list. The runtime subscribes it pre-spawn; the race is closed by construction.
- [ ] Capture `data["sink_invocation_id"] = build.sink_invocation_id` after `ctx.bazel.build` returns, then emit a running `task_update` so the Aspect Workflows link surfaces live.
- [ ] Call `setup_phase(ctx, lifecycle, subject, kind, data, hc_trait, bazel_trait, ...)` as the FIRST thing in `_impl` — it emits the Setup phase mark (the first `task_update`, which inits the status surfaces and renders their first body from `kind` + `data`), resolves Bazel flags, and runs `health_check` (a failed check concludes the surface and fails the task).
- [ ] Iterate `bazel_trait.build_start` / `build_event` / `build_end` so features fire (BK section markers, artifact upload).
- [ ] Emit a terminal `task_update` with `final=True` and a terminal `status` (`"passed"` / `"failed"` / `"warning"` / `"aborted"`) on every `return` path, and return the `TaskConclusion` it hands back. There is no separate `task_complete` hook.
- [ ] If a feature subscribes to `bazel_trait.build_event`, your BES loop must call those hooks per event — otherwise `_on_build_event` callbacks never run.
- [ ] For tasks that retry the bazel invocation, create a fresh `iterator()` handle per attempt — handles are single-use.

**State management** — see [State management: patterns and rules](#state-management-patterns-and-rules)

- [ ] **No data fields on traits.** Every trait field is a `Callable`, `list[Callable]`, or declarative config the task reads at dispatch time. If you wrote `attr(str, ...)` or `attr(dict, ...)` or `attr(list, ...)` on a trait, stop and pick Pattern 2 or Pattern 3 instead.
- [ ] **Task module doesn't reference features.** No `features = [...]` in the `Task(...)` constructor; no `load(".../feature/...", ...)` from a task module.
- [ ] **Cross-feature data goes through wrappers.** Callers write `tips.append(ctx, t)`, `artifacts.append(ctx, a)`, `rate_limit.record(ctx, ...)` — never `ctx.traits[X].some_method(...)` directly.
- [ ] **User-facing config goes on feature args**, not on trait fields. Templates, formatters, severity overrides, silenced-tip ids — none of these belong on a trait.
- [ ] **New libraries that need per-task state pick Pattern 2 or 3** based on whether the state has a single natural owner. Co-locate Pattern-2 owner features with their trait in `private/lib/<name>.axl`.

**Testing**

- [ ] Run the relevant snapshot suite (`./target/debug/aspect-cli dev test-<kind>-template-snapshots`) and `tests axl` before pushing.
