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
        │ features: Workflows, ArtifactUpload,         │
        │           GithubStatusChecks,                │
        │           GithubStatusComments,              │
        │           GithubLintComments,                │
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
        │   init_data() / render_check_output()     │
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
   - `data` — kind-specific data dict (the `init_data()` accumulator, see [Results dict shape](#results-dict-shape)).

   Tasks emit running updates as they make progress (e.g. lint emits one per SARIF report; build/test emit on every BES event that updates the failure / test-summary state). The two final values `"passed"` / `"failed"` are *terminal*: features take that as the cue to complete the check run / finalise the annotation, and the throttle in `lib/check_dispatch.should_emit_update` always lets terminals through.

3. **`task_complete(ctx, exit_code)`** — fires once with the integer exit code the task is about to return. Features subscribe here for cleanup that needs the exit code rather than the rendered status (e.g. updating telemetry counters, posting to follow-up systems).

The order at runtime, end-to-end:

```
lifecycle.task_started("//...")
  └─ GithubStatusChecks creates the check run
  └─ BuildkiteAnnotations posts the first "info" annotation
hc_trait.pre_health_check
  └─ Workflows prints `--- :computer: Workflows runner environment` / health check
bazel_trait.build_start
  └─ Workflows prints `--- :bazel: Running bazel <task> [<task-key>] <targets>`
ctx.bazel.build(...)
events = build.build_events()                    # subscribe BEFORE the next emit
data["sink_invocation_id"] = build.sink_invocation_id
lifecycle.task_update(running)                   # link surfaces in the annotation
for event in events:
    bazel_trait.build_event(ctx, event)          # ArtifactUpload records testlog paths
    if process_event(data, event):
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

| Trait                        | Defined in                                                                | Slots                                                                                                       | Purpose |
|------------------------------|---------------------------------------------------------------------------|-------------------------------------------------------------------------------------------------------------|---------|
| **`BazelTrait`**             | [bazel.axl](bazel.axl)                                                    | `build_start`, `build_event`, `build_end`, `build_retry`, `build_event_sinks`, `task_flags`, `flags`, `startup_flags`, `extra_flags`, `extra_startup_flags`, `execution_log_sinks` | Shape every Bazel invocation in the task: extra flags, BES sinks, per-event hooks, build-end cleanup. |
| **`HealthCheckTrait`**       | [lib/health_check.axl](lib/health_check.axl)                              | `pre_health_check`, `post_health_check`                                                                     | Pre-build runner-environment / agent-health checks; post-build verifications. |
| **`TaskLifecycleTrait`**     | [lib/lifecycle.axl](lib/lifecycle.axl)                                    | `task_started`, `task_update`, `task_complete`                                                              | The three lifecycle slots above. |
| **`ArtifactsTrait`**         | [lib/artifacts.axl](lib/artifacts.axl)                                    | `artifacts_browse_url`, `artifact_urls`, `testlogs_label_urls`                                              | Output: artifact URLs the upload feature populates and the renderers consume. |
| **`GitHubStatusChecksTrait`**| [feature/github_status_checks.axl](feature/github_status_checks.axl)      | `templates`, `metadata_keys`                                                                                | Per-task GHSC overrides. |
| **`GitHubCheckRunTrait`**    | [lib/checkrun.axl](lib/checkrun.axl)                                      | `html_url`                                                                                                  | Carries the created check run's `html_url` so sibling features (GHSComments) can link to it. |
| **`LintTrait`**              | [lint.axl](lint.axl)                                                      | `findings_destination`, `lint_start`, `lint_report`, `lint_patch`, `lint_end`, `changed_files`, `suggestions`, `comment_urls`, … | Lint-specific knobs and findings-destination routing. |
| **`DeliveryTrait`**          | [delivery.axl](delivery.axl)                                              | `delivery_start`, `deliver_target`, `delivery_end`                                                          | Delivery-specific knobs. |

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
data["sink_invocation_id"] = build.sink_invocation_id
for handler in lifecycle.task_update:
    handler(ctx, TaskUpdate(kind = "...", status = "running", data = data))

for event in events:                  # iterate the already-buffered channel
    ...
```

If you violate this rule, the failure mode is *intermittent* — cold-daemon runs work, warm-daemon runs fail — which is exactly the kind of bug that escapes local testing.

---

## Per-kind result libraries

Every task that surfaces data goes through a `lib/<kind>_results.axl` library that owns the rendering for that task type. The contract:

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

  ┌─ Per-kind extensions (added by lib/<kind>_results.init_data) ───┐
  │ # lint     → data["lint"]                                          │
  │   "diagnostics", "strategy", "build_failed", "linter_exit_code",   │
  │   "changed_files", "counts_by_severity", "counts_by_tool"          │
  │ # format   → data["format"]                                        │
  │   "scope", "formatter_target", "on_change_resolved",               │
  │   "affected_files", ...                                            │
  │ # gazelle  → data["gazelle"]                                       │
  │   "gazelle_target", "check_only", "on_change_resolved",            │
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

### Summary header rows

The `· `-separated rows above the **Last update** line (the same data on GH check-runs, BK annotations, and GHSC summary comments) are built by `bazel_results.build_invocation_rows`. They are grouped semantically — each row answers one question, kept short enough that the row fits on a single visual line in narrow surfaces.

| Row | Question it answers      | Items                                                                                                           |
|-----|--------------------------|-----------------------------------------------------------------------------------------------------------------|
| 1   | **What & when?**         | status icon + label · `📅` ISO date · `⏱` elapsed with phase breakdown                                          |
| 2   | **Who & what tools?**    | `👤` user · `🖥` build host · `💎` aspect-cli version (`:aspect:` on Buildkite)                                 |
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

- **`GithubStatusChecks`** authenticates against GitHub via the Aspect App, fetches the running job URL via the Actions REST API, posts annotations only on the *terminal* update (GitHub appends across PATCHes — running emits would duplicate findings), and supports lint annotations + multi-batch chunking.
- **`BuildkiteAnnotations`** probes for `buildkite-agent` on PATH, builds a leading task pill (`[:aspect: task <key>]`) so a step that runs multiple tasks gets distinguishable annotations, and maps `(kind, data, status)` to `--style success/info/warning/error` via `_severity_for`.

---

## Artifact uploads

`feature/artifacts.axl` registers two callbacks:

- `_on_build_event(ctx, event)` on `bazel_trait.build_event` — records testlog paths, profile output, etc. as the BES stream goes by.
- `_on_build_end(ctx, exit_code)` on `bazel_trait.build_end` — uploads the recorded files via the host CI's artifact API (GitHub Actions REST, `buildkite-agent artifact upload`, CircleCI store_artifacts, GitLab Generic Packages), then populates `ArtifactsTrait.artifacts_browse_url` / `artifact_urls` / `testlogs_label_urls`.

For these to fire, the task **must** iterate `bazel_trait.build_event` inside its BES loop and `bazel_trait.build_end` after `build.wait()`. Skip either and no upload runs. The canonical pattern is in the [Anatomy of a task](#anatomy-of-a-task-_impl) section below.

The check-run / annotation features read `ArtifactsTrait` via the shared `apply_artifact_links` helper. The renderers suppress the Artifacts row when both fields are empty, so a task that didn't upload anything stays clean.

---

## Anatomy of a task `_impl`

The canonical flow, with annotations. Use it as a checklist when reading or writing a task:

```python
def _impl(ctx: TaskContext) -> int:
    bazel_trait = ctx.traits[BazelTrait]
    hc_trait    = ctx.traits[HealthCheckTrait]
    lifecycle   = ctx.traits[TaskLifecycleTrait]

    # 1. Build the data dict and fire task_started FIRST so any later
    #    failure (a pre-build hook, a config error) still produces a
    #    visible status check / annotation.
    data = init_data()
    data["start_time_ms"]  = now_ms(ctx)
    data["target_pattern"] = " ".join(ctx.args.targets)
    ... task-specific bookkeeping ...
    for handler in lifecycle.task_started:
        handler(ctx, data["target_pattern"])   # subject (str) shown in the title

    # 2. Pre-health checks. The `Workflows` feature registers
    #    `print_environment_info` + `agent_health_check` here, which print
    #    the `--- :computer: Workflows runner environment` and health check
    #    BK section markers when running on an Aspect Workflows runner.
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

    # 5. Fire build_start hooks BEFORE spawning Bazel. The `Workflows`
    #    feature registers the `--- :bazel: Running bazel <task> [<key>]
    #    <targets>` BK section marker here.
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
    data["sink_invocation_id"] = build.sink_invocation_id
    for handler in lifecycle.task_update:
        handler(ctx, TaskUpdate(
            kind   = "<task>_results",
            status = "running",
            data   = data,
        ))

    # 8. Drain the event iterator. Two things per event:
    #    - bazel_trait.build_event hooks (ArtifactUpload records testlog
    #      paths; user features can hook here too)
    #    - process_event(data, event) to populate the bazel state +
    #      stream metadata into the live annotation
    for event in events:
        for handler in bazel_trait.build_event:
            handler(ctx, event)
        if process_event(data, event):
            for handler in lifecycle.task_update:
                handler(ctx, TaskUpdate(
                    kind   = "<task>_results",
                    status = "running",
                    data   = data,
                ))

    # 9. Wait for bazel, then fire build_end hooks. ArtifactUpload's
    #    `_on_build_end` runs the actual upload step here.
    build_status = build.wait()
    for hook in bazel_trait.build_end:
        hook(ctx, build_status.code)

    if not build_status.success:
        # Emit terminal task_update + task_complete BEFORE returning.
        data["<kind>"]["build_failed"] = True   # kind-specific failure flag
        for handler in lifecycle.task_update:
            handler(ctx, TaskUpdate(
                kind   = "<task>_results",
                status = "failed",
                data   = data,
            ))
        for handler in lifecycle.task_complete:
            handler(ctx, 1)
        return 1

    # 10. ... task-specific post-build work (run binary, parse output, etc.)

    # 11. Terminal task_update + task_complete + post_health_check.
    status = "passed" if exit_code == 0 else "failed"
    if not data.get("finish_time_ms"):
        data["finish_time_ms"] = now_ms(ctx)
    if data.get("start_time_ms") and not data["bazel"].get("wall_time_ms"):
        data["bazel"]["wall_time_ms"] = data["finish_time_ms"] - data["start_time_ms"]
    for handler in lifecycle.task_update:
        handler(ctx, TaskUpdate(
            kind   = "<task>_results",
            status = status,
            data   = data,
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
2. If your task drives Bazel, declare `BazelTrait` in the task `traits = [...]`. If it surfaces data in CI, declare `TaskLifecycleTrait`. If it should print runner-env / health-check sections on Buildkite, declare `HealthCheckTrait`. If it uploads artifacts, declare `ArtifactsTrait`.
3. Decide the `task_update.kind`. If your task's data shape matches an existing renderer (build/test → `bazel_results`, lint → `lint_results`, etc.), reuse it. Otherwise add a new kind — see below.
4. Wire the task in `MODULE.aspect`'s task registry.
5. Add a row to [`README.md`](README.md).

You get GHSC + BK annotations + artifact uploads for free as long as you (a) iterate the trait hooks listed in the canonical flow, (b) emit the right `TaskUpdate.kind`, and (c) capture `sink_invocation_id` after `ctx.bazel.build(...)`.

---

## How to add a new task kind (new renderer)

If your task's data shape doesn't fit any existing kind:

1. Create `lib/<kind>_results.axl` exporting:
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
./target/debug/aspect-cli dev test-pr-comment-snapshots        # github_status_comments aggregated PR comment
```

These run end-to-end through `render_check_output` — they exercise the actual templates, so any layout regression shows up as a visible diff in the printed scenario.

### `tests axl` — AXL unit tests

`./target/debug/aspect-cli tests axl` runs all `test_case(tc, ..., "label")` calls in `.aspect/axl.axl` (~720+ cases). Add cases there for new helpers in `lib/*.axl`.

### Live CI

Each task is exercised on every CI provider (GitHub Actions, Buildkite, CircleCI, GitLab) via the per-task pipeline definitions in [`.buildkite/pipeline.yaml`](../../../../../.buildkite/pipeline.yaml), [`.github/workflows/ci.yaml`](../../../../../.github/workflows/ci.yaml), [`.gitlab-ci.yml`](../../../../../.gitlab-ci.yml), and [`.circleci/config.yml`](../../../../../.circleci/config.yml). Each task runs twice per CI host: once with `ASPECT_DEBUG=1` (the debug variant) and once without — debug emits the `target: "axl.log"` trace lines that the per-task `_impl` printers go to, which is the fastest way to diagnose live failures.

When a task you change works in snapshots but fails live (artifact URL malformed, BK section missing, etc.), capture the `ASPECT_DEBUG=1` step's stdout from the BK / GHA UI — every helper logs enough state to pin down the failure mode without re-running locally.

---

## Quick checklist for new contributors

When writing a new task or modifying an existing one, sanity-check:

- [ ] Subscribe to BES events (`build.build_events()`) **immediately** after `ctx.bazel.build(...)` returns, before any other work.
- [ ] Capture `data["sink_invocation_id"] = build.sink_invocation_id` right after, then emit a running `task_update` so the Aspect Workflows link surfaces live.
- [ ] Iterate `bazel_trait.build_start` / `build_event` / `build_end` so features fire (BK section markers, artifact upload).
- [ ] Iterate `hc_trait.pre_health_check` / `post_health_check` so the runner-env sections render and post-failures surface.
- [ ] Emit terminal `task_update` (`status="passed"` or `"failed"`) and `task_complete(exit_code)` *before* every `return`.
- [ ] If a feature subscribes to `bazel_trait.build_event`, your BES loop must call those hooks per event — otherwise `_on_build_event` callbacks never run.
- [ ] If your task rebuilds or retries, the broadcaster's per-attempt subscriber is fresh; don't reuse the iterator across attempts.
- [ ] Run the relevant snapshot suite (`./target/debug/aspect-cli dev test-<kind>-template-snapshots`) and `tests axl` before pushing.
