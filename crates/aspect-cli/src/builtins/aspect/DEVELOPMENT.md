# Developing built-in tasks

This is the contributor guide for **`crates/aspect-cli/src/builtins/aspect/`** — the AXL module that ships with `aspect-cli` and provides the `build` / `test` / `lint` / `format` / `gazelle` / `delivery` tasks. If you only want to know what each task *does*, read [`README.md`](README.md) first.

This guide explains *why* the existing tasks look the way they do and how to keep new tasks consistent with them. It covers:

1. [Architecture at a glance](#architecture-at-a-glance)
2. [The per-task lifecycle](#the-per-task-lifecycle)
3. [Traits the framework exposes](#traits-the-framework-exposes)
4. [BES streaming and the broadcaster race](#bes-streaming-and-the-broadcaster-race)
5. [Per-kind result libraries](#per-kind-result-libraries)
6. [Status checks and annotations](#status-checks-and-annotations)
7. [Artifact uploads](#artifact-uploads)
8. [Anatomy of a task `_impl`](#anatomy-of-a-task-_impl) — the canonical flow with annotated code
9. [How to add a new task](#how-to-add-a-new-task)
10. [How to add a new task kind (new renderer)](#how-to-add-a-new-task-kind-new-renderer)
11. [Testing](#testing)

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
              │   hc_trait.pre_health_check /                    │
              │     hc_trait.post_health_check                   │
              │   lifecycle.task_started / task_update /         │
              │     task_complete                                │
              └────────────┬─────────────────┬──────────────────┘
                           │                 │
              registered by │                 │ consumed by
                           ▼                 ▼
        ┌──────────────────────────────────────────────┐
        │ features: BazelDefaults, ArtifactUpload,     │
        │           GithubStatusChecks,                │
        │           BuildkiteAnnotations, …            │
        └────────────┬─────────────────────────────────┘
                     │ render via dispatch table in
                     ▼
        ┌──────────────────────────────────────────────┐
        │ lib/check_dispatch.axl                       │
        │   RENDERERS = {bazel, lint, format,          │
        │                gazelle, delivery}            │
        └────────────┬─────────────────────────────────┘
                     │ each entry points at
                     ▼
        ┌──────────────────────────────────────────────┐
        │ lib/<kind>_results.axl                       │
        │   init_results() / render_check_output()     │
        └──────────────────────────────────────────────┘
```

Tasks own the *flow* (which Bazel command runs when, what extra processing happens around it). Features own the *cross-cutting concerns* (status checks, annotations, artifact uploads, runner health). Result libraries own the *rendering*. Tasks never poke at GitHub or Buildkite directly — they fire lifecycle events and the features react.

---

## The per-task lifecycle

`TaskLifecycleTrait` (defined in [`traits.axl`](traits.axl)) defines three slots that every task fires in order:

1. **`task_started(ctx, subject)`** — fires once at the start of the task, before any health check or build. The `subject` is whatever the task wants to display in the rendered title (e.g. the target pattern for build/test/lint, the formatter target for format, the gazelle target for gazelle, the delivery targets for delivery).

   Features that subscribe here typically post the *initial* "running" surface (a creating GitHub check run, a first BK annotation in `--style info`).

2. **`task_update(ctx, TaskUpdate)`** — fires zero or more times during the task. Each `TaskUpdate` carries:
   - `kind` — the result-library identifier (e.g. `"lint_results"`). Drives renderer dispatch.
   - `status` — `"running"`, `"failing"`, `"passed"`, or `"failed"`.
   - `data` — typically `{"results": <kind-specific-results-dict>}`.

   Tasks emit running updates as they make progress (e.g. lint emits one per SARIF report; build/test emit on every BES event that updates the failure / test-summary state). The two final values `"passed"` / `"failed"` are *terminal*: features take that as the cue to complete the check run / finalise the annotation, and the throttle in `lib/check_dispatch.should_emit_update` always lets terminals through.

3. **`task_complete(ctx, exit_code)`** — fires once with the integer exit code the task is about to return. Features subscribe here for cleanup that needs the exit code rather than the rendered status (e.g. updating telemetry counters, posting to follow-up systems).

The order at runtime, end-to-end:

```
lifecycle.task_started("//...")
  └─ GithubStatusChecks creates the check run
  └─ BuildkiteAnnotations posts the first "info" annotation
hc_trait.pre_health_check
  └─ BazelDefaults prints `--- :aspect: Workflows Runner Environment` / Health Check
bazel_trait.build_start
  └─ BazelDefaults prints `--- :bazel: Running bazel <task> [<task-key>] <targets>`
ctx.bazel.build(...)
events = build.build_events()                    # subscribe BEFORE the next emit
results["sink_invocation_id"] = build.sink_invocation_id
lifecycle.task_update(running)                   # link surfaces in the annotation
for event in events:
    bazel_trait.build_event(ctx, event)          # ArtifactUpload records testlog paths
    if process_event(results, event):
        lifecycle.task_update(running)           # streamed metadata + targets
build_status = build.wait()
bazel_trait.build_end(ctx, build_status.code)    # ArtifactUpload uploads
... task-specific work (run formatter, parse SARIF, etc.) ...
lifecycle.task_update(passed | failed)           # terminal — final body
lifecycle.task_complete(exit_code)
hc_trait.post_health_check
return exit_code
```

---

## Traits the framework exposes

| Trait                        | Defined in                       | Slots                                                                                                       | Purpose |
|------------------------------|----------------------------------|-------------------------------------------------------------------------------------------------------------|---------|
| **`BazelTrait`**             | [traits.axl](traits.axl)         | `build_start`, `build_event`, `build_end`, `build_retry`, `build_event_sinks`, `task_flags`, `flags`, `startup_flags`, `extra_flags`, `extra_startup_flags`, `execution_log_sinks` | Shape every Bazel invocation in the task: extra flags, BES sinks, per-event hooks, build-end cleanup. |
| **`HealthCheckTrait`**       | [traits.axl](traits.axl)         | `pre_health_check`, `post_health_check`                                                                     | Pre-build runner-environment / agent-health checks; post-build verifications. |
| **`TaskLifecycleTrait`**     | [traits.axl](traits.axl)         | `task_started`, `task_update`, `task_complete`                                                              | The three lifecycle slots above. |
| **`ArtifactsTrait`**         | [traits.axl](traits.axl)         | `artifacts_browse_url`, `artifact_urls`, `testlogs_label_urls`                                              | Output: artifact URLs the upload feature populates and the renderers consume. |
| **`GitHubStatusChecksTrait`**| [traits.axl](traits.axl)         | `templates`, `metadata_keys`, `enabled`, …                                                                  | Per-task GHSC overrides. |
| **`LintTrait`**              | [lint.axl](lint.axl)             | `findings_destination`, `lint_start`, `lint_report`, `changed_files`, …                                     | Lint-specific knobs. |
| **`DeliveryTrait`**          | [delivery.axl](delivery.axl)     | `delivery_start`, …                                                                                         | Delivery-specific knobs. |

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

## BES streaming and the broadcaster race

A subtle but load-bearing design detail. The runtime exposes BES events via a *broadcaster* (see [`crates/axl-runtime/src/engine/bazel/stream/broadcaster.rs`](../../../../axl-runtime/src/engine/bazel/stream/broadcaster.rs)):

- Subscribers register and get a private channel. The broadcaster dispatches every event to every subscriber's channel.
- **Subscribers that join after an event has been broadcast do *not* receive that event.** There is no replay buffer.

`ctx.bazel.build(...)` registers two subscribers internally before returning (a tracing sink and the gRPC sinks from `bazel_trait.build_event_sinks`). User code that wants its own iterator calls `build.build_events()`, which calls `event_stream.subscribe()` lazily.

On a **warm bazel daemon** with a fully cached build, BES events stream within milliseconds. If the task does any meaningful work between `ctx.bazel.build(...)` returning and `build.build_events()` being called — including emitting a running `task_update` (which spawns `buildkite-agent annotate`, dozens of ms) — the user-side subscriber registers too late and the early burst (`build_started`, `target_completed`, `named_set_of_files`) is gone. `runnable.determine_entrypoint()` then can't find the target's executable, format/gazelle silently can't run their binary, lint can't read SARIF reports.

**The rule: call `build.build_events()` immediately after `ctx.bazel.build(...)` returns, before any other work.** Stash the iterator and use it later:

```python
build = ctx.bazel.build(...)
events = build.build_events()        # ← subscribe FIRST, the channel is now buffering

# Now safe to do potentially-slow work — the broadcaster is feeding `events`'
# channel from t=0 regardless of how slow we are below.
results["sink_invocation_id"] = build.sink_invocation_id
for handler in lifecycle.task_update:
    handler(ctx, TaskUpdate(kind = "...", status = "running",
                            data = {"results": results}))

for event in events:                  # iterate the already-buffered channel
    ...
```

If you violate this rule, the failure mode is *intermittent* — cold-daemon runs work, warm-daemon runs fail — which is exactly the kind of bug that escapes local testing. There is a record of one full bug-hunt iteration of this in `/Users/greg/aspect/claude-6/build_events_subscribe_race.md`.

---

## Per-kind result libraries

Every task that surfaces results goes through a `lib/<kind>_results.axl` library that owns the rendering for that task type. The contract:

```python
def init_results():
    """Return a fresh results dict. Should derive from bazel_results.init_results()
    so process_event() can populate the full bazel state."""

def render_check_output(ctx, results, status, render_ctx, links,
                        templates = None, metadata_keys = None):
    """Return {title: str, summary: str, text: str} suitable for both a GitHub
    check-run output and a Buildkite annotation body."""
```

The lint / format / gazelle / delivery libraries all start their `init_results()` from `bazel_results.init_results()`:

```python
load("./bazel_results.axl", bazel_init_results = "init_results")

def init_results():
    r = bazel_init_results()
    r.update({
        "diagnostics":    [],         # task-specific
        ...
    })
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

Result: every task — even gazelle and format — renders the same Targets / Build Metrics / Invocation / Aspect Workflows Runner / Workspace Status / Build Metadata / Options-parsed detail body at the bottom. Adding a metadata row to `bazel_results` automatically appears across every task type with no per-kind change.

---

## Status checks and annotations

[`lib/check_dispatch.axl`](lib/check_dispatch.axl) holds the parts both `GithubStatusChecks` and `BuildkiteAnnotations` would otherwise duplicate:

```python
load("./lib/check_dispatch.axl",
    "RENDERERS",          # kind → struct(init, render)
    "renderer_for_kind",
    "kind_for_task",      # task_name → kind for the initial pre-update render
    "to_display_name",
    "make_render_ctx",    # build the render_ctx dict from feature state
    "apply_artifact_links",  # copy ArtifactsTrait fields onto links
    "should_emit_update",    # throttle non-final updates, always pass terminals
)
```

When a new task kind is added (see [below](#how-to-add-a-new-task-kind-new-renderer)), the only change to the dispatch is a single entry in `lib/check_dispatch.axl`'s `RENDERERS` table. Both features pick it up automatically.

Feature-local responsibilities (which can't be lifted because the surfaces differ):

- **`GithubStatusChecks`** authenticates against GitHub via the Aspect App, fetches the running job URL via the Actions REST API, posts annotations only on the *terminal* update (GitHub appends across PATCHes — running emits would duplicate findings), and supports lint annotations + multi-batch chunking.
- **`BuildkiteAnnotations`** probes for `buildkite-agent` on PATH, builds a leading task pill (`[:aspect: task <key>]`) so a step that runs multiple tasks gets distinguishable annotations, and maps `(kind, results, status)` to `--style success/info/warning/error` via `_severity_for`.

---

## Artifact uploads

`feature/artifacts.axl` registers two callbacks:

- `_on_build_event(ctx, event)` on `bazel_trait.build_event` — records testlog paths, profile output, etc. as the BES stream goes by.
- `_on_build_end(ctx, exit_code)` on `bazel_trait.build_end` — uploads the recorded files via the host CI's artifact API (GitHub Actions REST, `buildkite-agent artifact upload`, CircleCI store_artifacts, GitLab Generic Packages), then populates `ArtifactsTrait.artifacts_browse_url` / `artifact_urls` / `testlogs_label_urls`.

For these to fire, the task **must** iterate `bazel_trait.build_event` inside its BES loop and `bazel_trait.build_end` after `build.wait()`. If you skip those iterations no upload runs (this was a real bug — format and gazelle were missing both for a while). The canonical pattern is in the [Anatomy of a task](#anatomy-of-a-task-_impl) section below.

The check-run / annotation features read `ArtifactsTrait` via the shared `apply_artifact_links` helper. The renderers suppress the Artifacts row when both fields are empty, so a task that didn't upload anything stays clean.

---

## Anatomy of a task `_impl`

The canonical flow, with annotations. Use it as a checklist when reading or writing a task:

```python
def _impl(ctx: TaskContext) -> int:
    bazel_trait = ctx.traits[BazelTrait]
    hc_trait    = ctx.traits[HealthCheckTrait]
    lifecycle   = ctx.traits[TaskLifecycleTrait]

    # 1. Build the results dict and fire task_started FIRST so any later
    #    failure (a pre-build hook, a config error) still produces a
    #    visible status check / annotation.
    results = init_results()
    results["start_time_ms"]  = now_ms(ctx)
    ... task-specific bookkeeping ...
    for handler in lifecycle.task_started:
        handler(ctx, ctx.args.targets)   # subject = what to show in the title

    # 2. Pre-health checks. BazelDefaults registers
    #    `print_environment_info` + `agent_health_check` here, which print
    #    the `--- :aspect: Workflows Runner Environment` and Health Check
    #    BK section markers. Skipping this means those sections are
    #    missing on the BK step page — visible regression.
    for hook in hc_trait.pre_health_check:
        hook(ctx)

    # 3. Compose flags. Trait `extra_flags` first, then `task_flags`
    #    callbacks (which need ctx.task.{name,key} so they have to run at
    #    invocation time), then user-supplied --bazel-flag values.
    flags = list(ctx.args.bazel_flags)
    flags.extend(bazel_trait.extra_flags)
    for hook in bazel_trait.task_flags:
        flags.extend(hook(ctx))
    if bazel_trait.flags:
        flags = bazel_trait.flags(flags)
    startup_flags = list(ctx.args.bazel_startup_flags)
    startup_flags.extend(bazel_trait.extra_startup_flags)
    if bazel_trait.startup_flags:
        startup_flags = bazel_trait.startup_flags(startup_flags)
    ctx.bazel.startup_flags.extend(startup_flags)

    # 4. Pass build_event_sinks (gRPC sinks the runtime registers BEFORE
    #    `Build::spawn` returns) so BES events reach the Aspect backend
    #    and the "Aspect Workflows" link resolves to a real invocation.
    build_events = list(bazel_trait.build_event_sinks) if bazel_trait.build_event_sinks else True

    # 5. Fire build_start hooks BEFORE spawning Bazel. BazelDefaults
    #    registers the `--- :bazel: Running bazel <task> [<key>] <targets>`
    #    BK section marker here.
    for hook in bazel_trait.build_start:
        hook(ctx)

    build = ctx.bazel.build(
        flags = flags,
        build_events = build_events,
        *ctx.args.targets,
    )

    # 6. Subscribe to the BES stream IMMEDIATELY after Build::spawn
    #    returns — BEFORE doing any potentially-slow work. See the
    #    "BES streaming and the broadcaster race" section above.
    events = build.build_events()

    # 7. Capture sink_invocation_id (the runtime mints it inside
    #    Build::spawn before returning, so it's available now without
    #    waiting for build.wait()). Emit a running task_update so the
    #    "Aspect Workflows" link surfaces in the live annotation as
    #    soon as the build is spawned.
    results["sink_invocation_id"] = build.sink_invocation_id
    for handler in lifecycle.task_update:
        handler(ctx, TaskUpdate(
            kind   = "<task>_results",
            status = "running",
            data   = {"results": results},
        ))

    # 8. Drain the event iterator. Two things per event:
    #    - bazel_trait.build_event hooks (ArtifactUpload records testlog
    #      paths; user features can hook here too)
    #    - process_event(results, event) to populate the bazel state +
    #      stream metadata into the live annotation
    for event in events:
        for handler in bazel_trait.build_event:
            handler(ctx, event)
        if process_event(results, event):
            for handler in lifecycle.task_update:
                handler(ctx, TaskUpdate(
                    kind   = "<task>_results",
                    status = "running",
                    data   = {"results": results},
                ))

    # 9. Wait for bazel, then fire build_end hooks. ArtifactUpload's
    #    `_on_build_end` runs the actual upload step here.
    build_status = build.wait()
    for hook in bazel_trait.build_end:
        hook(ctx, build_status.code)

    if not build_status.success:
        # Emit terminal task_update + task_complete BEFORE returning.
        results["build_failed"] = True
        for handler in lifecycle.task_update:
            handler(ctx, TaskUpdate(
                kind   = "<task>_results",
                status = "failed",
                data   = {"results": results},
            ))
        for handler in lifecycle.task_complete:
            handler(ctx, 1)
        return 1

    # 10. ... task-specific post-build work (run binary, parse output, etc.)

    # 11. Terminal task_update + task_complete + post_health_check.
    status = "passed" if exit_code == 0 else "failed"
    if not results.get("finish_time_ms"):
        results["finish_time_ms"] = now_ms(ctx)
    if results.get("start_time_ms") and not results.get("wall_time_ms"):
        results["wall_time_ms"] = results["finish_time_ms"] - results["start_time_ms"]
    for handler in lifecycle.task_update:
        handler(ctx, TaskUpdate(
            kind   = "<task>_results",
            status = status,
            data   = {"results": results},
        ))
    for handler in lifecycle.task_complete:
        handler(ctx, exit_code)
    for hook in hc_trait.post_health_check:
        result = hook(ctx)
        if result != None:
            fail(result)
    return exit_code
```

The `lint.axl` impl is the closest to this template; `format.axl` and `gazelle.axl` are similar but pass a single target instead of a list and build the entrypoint in step 10. `delivery.axl` does multiple Bazel invocations (one per phase) — the same pattern just runs once per phase.

---

## How to add a new task

1. Create `crates/aspect-cli/src/builtins/aspect/<task_name>.axl` and follow the [canonical flow](#anatomy-of-a-task-_impl).
2. If your task drives Bazel, declare `BazelTrait` in the task `traits = [...]`. If it surfaces results in CI, declare `TaskLifecycleTrait`. If it should print runner-env / health-check sections on Buildkite, declare `HealthCheckTrait`. If it uploads artifacts, declare `ArtifactsTrait`.
3. Decide the `task_update.kind`. If your task's results shape matches an existing renderer (build/test → `bazel_results`, lint → `lint_results`, etc.), reuse it. Otherwise add a new kind — see below.
4. Wire the task in `MODULE.aspect`'s task registry.
5. Add a row to [`README.md`](README.md).

You get GHSC + BK annotations + artifact uploads for free as long as you (a) iterate the trait hooks listed in the canonical flow, (b) emit the right `TaskUpdate.kind`, and (c) capture `sink_invocation_id` after `ctx.bazel.build(...)`.

---

## How to add a new task kind (new renderer)

If your task's results shape doesn't fit any existing kind:

1. Create `lib/<kind>_results.axl` exporting:
   ```python
   def init_results():
       r = bazel_init_results()
       r.update({...task-specific fields...})
       return r

   def render_check_output(ctx, results, status, render_ctx, links,
                           templates = None, metadata_keys = None):
       return {"title": ..., "summary": ..., "text": ...}
   ```
2. Append `bazel_results.SHARED_DETAILS_BODY_TEMPLATE` to your `_DETAILS_TEMPLATE` so the rendered body has the same Targets / Build Metrics / Invocation / etc. tail as every other task.
3. In `lib/check_dispatch.axl`, add an entry to `RENDERERS` and (if your task name should resolve to this kind for the *initial* pre-update render) to `_TASK_KIND_DEFAULTS`.
4. Add a snapshot test at `lib/<kind>_results_test.axl` mirroring the existing ones.
5. Register the snapshot test as a task in `.aspect/config.axl` so `aspect dev test-<kind>-template-snapshots` runs it.

Both `GithubStatusChecks` and `BuildkiteAnnotations` will dispatch to your new renderer automatically — no per-feature changes.

---

## Testing

Three layers, listed in increasing fidelity:

### Snapshot tests (per-kind result libraries)

Each `lib/<kind>_results_test.axl` enumerates representative scenarios (clean / partial / full failure / aborted / large-trim / etc.) and prints the rendered output. Run with:

```sh
./target/debug/aspect-cli dev test-template-snapshots          # bazel_results
./target/debug/aspect-cli dev test-lint-template-snapshots
./target/debug/aspect-cli dev test-format-template-snapshots
./target/debug/aspect-cli dev test-gazelle-template-snapshots
./target/debug/aspect-cli dev test-delivery-template-snapshots
./target/debug/aspect-cli dev test-bk-annotation-snapshots     # BK pill + body
```

These run end-to-end through `render_check_output` — they exercise the actual templates, so any layout regression shows up as a visible diff in the printed scenario.

### `tests axl` — AXL unit tests

`./target/debug/aspect-cli tests axl` runs all `test_case(tc, ..., "label")` calls in `.aspect/axl.axl` (~700 cases as of writing). Add cases there for new helpers in `lib/*.axl`.

### Live CI

Each task is exercised on every CI provider (GitHub Actions, Buildkite, CircleCI, GitLab) via the per-task pipeline definitions in [`.buildkite/pipeline.yaml`](../../../../../.buildkite/pipeline.yaml), [`.github/workflows/ci.yaml`](../../../../../.github/workflows/ci.yaml), [`.gitlab-ci.yml`](../../../../../.gitlab-ci.yml), and [`.circleci/config.yml`](../../../../../.circleci/config.yml). Each task runs twice per CI host: once with `ASPECT_DEBUG=1` (the debug variant) and once without — debug emits the `target: "axl.log"` trace lines that the per-task `_impl` printers go to, which is the fastest way to diagnose live failures.

When a task you change works in snapshots but fails live (artifact URL malformed, BK section missing, etc.), capture the `ASPECT_DEBUG=1` step's stdout from the BK / GHA UI — every helper logs enough state to pin down the failure mode without re-running locally.

---

## Quick checklist for new contributors

When writing a new task or modifying an existing one, sanity-check:

- [ ] Subscribe to BES events (`build.build_events()`) **immediately** after `ctx.bazel.build(...)` returns, before any other work.
- [ ] Capture `results["sink_invocation_id"] = build.sink_invocation_id` right after, then emit a running `task_update` so the Aspect Workflows link surfaces live.
- [ ] Iterate `bazel_trait.build_start` / `build_event` / `build_end` so features fire (BK section markers, artifact upload).
- [ ] Iterate `hc_trait.pre_health_check` / `post_health_check` so the runner-env sections render and post-failures surface.
- [ ] Emit terminal `task_update` (`status="passed"` or `"failed"`) and `task_complete(exit_code)` *before* every `return`.
- [ ] If a feature subscribes to `bazel_trait.build_event`, your BES loop must call those hooks per event — otherwise `_on_build_event` callbacks never run.
- [ ] If your task rebuilds or retries, the broadcaster's per-attempt subscriber is fresh; don't reuse the iterator across attempts.
- [ ] Run the relevant snapshot suite (`./target/debug/aspect-cli dev test-<kind>-template-snapshots`) and `tests axl` before pushing.
