# AXL + Aspect Workflows

This document explains how Aspect CLI's AXL scripting layer integrates with the
[Aspect Workflows](https://docs.aspect.build/workflows) runner platform.

## Overview

Workflows integration is not a separate subsystem. It is woven into the standard
AXL task/fragment model. When a job runs on a Workflows runner, the CLI detects
this via environment variables and transparently activates runner-specific
behavior: remote cache, Build Event Service (BES) forwarding, artifact upload,
health checks, and artifact delivery. User `.axl` config files can extend or
override any of these behaviors through the fragment API.

```
Workflows Runner (EC2/GCE instance)
  │
  │  ASPECT_WORKFLOWS_* env vars
  ▼
aspect CLI
  ├── config phase  ──► builtins.axl + artifacts.axl + delivery.axl
  │                      └─ fragments configured from env vars
  └── task phase   ──► build.axl / test.axl / delivery.axl
                        └─ fragments drive hooks at each lifecycle stage
```

---

## Source Layout

All builtin Workflows behavior lives under:

```
crates/aspect-cli/src/builtins/aspect/
├── fragments.axl          # Fragment type definitions
├── build.axl              # `build` task implementation
├── test.axl               # `test` task implementation
├── delivery.axl           # `delivery` task implementation
├── bazel.axl              # Bazel helper utilities
├── config/
│   ├── builtins.axl       # Runner detection; wires BazelFragment + HealthCheckFragment
│   ├── artifacts.axl      # Wires artifact upload into BazelFragment.build_event
│   └── delivery.axl       # Registers the `delivery` task
└── lib/
    ├── environment.axl    # Runner/CI env var parsing; flag generation
    ├── health_check.axl   # Cache warming + health check logic
    ├── artifacts.axl      # CI-platform artifact upload (GHA, Buildkite, GitLab, CircleCI)
    ├── deliveryd.axl      # HTTP client for the deliveryd Unix socket
    ├── build_metadata.axl # Git + CI metadata → --build_metadata flags
    └── tar.axl            # bsdtar wrapper for test log archiving
```

---

## Fragment Types

Defined in `fragments.axl`, three fragments carry all Workflows-related state
and hooks. They are global singletons: one instance per type, shared by all
tasks that opt in.

### `BazelFragment`

Controls how `bazel build` and `bazel test` are invoked.

| Field | Type | Description |
|-------|------|-------------|
| `extra_flags` | `list[str]` | Appended to every Bazel invocation |
| `extra_startup_flags` | `list[str]` | Prepended before the Bazel command |
| `build_event_sinks` | `list[BuildEventSink]` | BES sinks (e.g., Bessie gRPC backend) |
| `execution_log_sinks` | `list[ExecLogSink]` | Execution log destinations |
| `flags` | `Callable[[list[str]], list[str]] \| None` | Transform the full flags list |
| `startup_flags` | `Callable[[list[str]], list[str]] \| None` | Transform startup flags |
| `build_start` | `list[Callable[[TaskContext, dict], None]]` | Called once before `bazel` runs |
| `build_event` | `list[Callable[[TaskContext, dict, dict], None]]` | Called per BES event during the build |
| `build_retry` | `Callable[[int], bool]` | Return `True` to retry on flakiness |
| `build_end` | `list[Callable[[TaskContext, dict, int], None]]` | Called once after `bazel` exits |

`config/builtins.axl` populates `extra_flags` and `build_event_sinks` when
`ASPECT_WORKFLOWS_REMOTE_CACHE` and `ASPECT_WORKFLOWS_BES_BACKEND` are set.
`config/artifacts.axl` appends a `build_event` hook that collects test outputs
and uploads them to the CI platform.

### `HealthCheckFragment`

Provides lifecycle hooks that run around every task execution.

| Field | Type | Description |
|-------|------|-------------|
| `pre_health_check` | `list[Callable[[TaskContext], None]]` | Runs before the task body |
| `post_health_check` | `list[Callable[[TaskContext], str \| None]]` | Runs after; non-`None` return fails the task |

`config/builtins.axl` registers three hooks when on a Workflows runner:
1. **Warming wait** — blocks until the cache warm is complete, then prints the
   result or a link to bootstrap logs if it failed.
2. **Last health check display** — prints the previous health check JSON stored
   at `ASPECT_WORKFLOWS_RUNNER_LAST_HEALTH_CHECK_FILE`.
3. **Bazel health check** — calls `ctx.bazel.health_check()` and runs
   `/etc/aspect/workflows/bin/signal_instance_unhealthy` if it fails.

### `DeliveryFragment`

Lifecycle hooks for the `delivery` task.

| Field | Type | Description |
|-------|------|-------------|
| `delivery_start` | `Callable[[], None]` | Called before delivering any target |
| `delivery_end` | `Callable[[], None]` | Called after all targets are processed |
| `deliver_target` | `Callable[[str, bool], None]` | Called per target (`label`, `is_forced`) |

---

## Runner Detection

`lib/environment.axl` reads environment variables and returns structured records
that the config files consume.

**Presence check:**
```starlark
is_workflows_runner = env.var("ASPECT_WORKFLOWS_RUNNER") != ""
```

**`Runner` record** (from `ASPECT_WORKFLOWS_RUNNER_*` vars):
- `storage_path` — ephemeral disk mount (default `/mnt/ephemeral`)
- `product_version`, `instance_id`, `instance_name`, `instance_type`
- `account`, `region`, `az`, `preemptible`
- `warming_enabled`, `warming_complete`, `warming_current_cache`
- `runner_job_history`, `last_health_check`
- `has_nvme_storage`

**`RemoteCache` record:**
- `endpoint` — `ASPECT_WORKFLOWS_REMOTE_CACHE` (gRPC cache address)
- `address` — `ASPECT_WORKFLOWS_REMOTE_BYTESTREAM_URI_PREFIX`

**`BuildEvents` record:**
- `backend` — `ASPECT_WORKFLOWS_BES_BACKEND` (Bessie gRPC endpoint)
- `results_url` — `ASPECT_WORKFLOWS_BES_RESULTS_URL`

---

## Configuration Phase

When `aspect build` (or any task) is invoked, the CLI runs all `.axl` files
under `config/` before executing the task. The three builtin config files fire
in order:

### `config/builtins.axl`

1. Reads the runner environment via `lib/environment.axl`.
2. If on a Workflows runner:
   - Adds `--remote_cache=<endpoint>` and related TLS flags to `BazelFragment.extra_flags`.
   - Creates a gRPC BES sink pointing at `BuildEvents.backend` and appends it to
     `BazelFragment.build_event_sinks`.
   - Appends `--build_metadata=...` flags from `lib/build_metadata.axl` (commit
     SHA, author, branch, CI platform, etc.).
3. Registers health check hooks into `HealthCheckFragment`.

### `config/artifacts.axl`

1. Detects the CI platform (GitHub Actions, Buildkite, GitLab, CircleCI) from
   token environment variables.
2. Appends a `build_event` hook to `BazelFragment` that:
   - Collects `test.log` and `test.xml` files from BES `TestResult` events.
   - Uploads them in batches (every 5 new files) via the platform API.
3. Also appends a `build_end` hook to flush any remaining queued uploads.

### `config/delivery.axl`

Registers the `delivery` task by calling `ctx.tasks.add(delivery_task)`, making
it available as `aspect delivery`.

---

## Task Execution Phase

### `build.axl` and `test.axl`

Both tasks follow the same pattern:

```
pre_health_check hooks
    │
    ▼
post_health_check hooks   (abort if any returns non-None)
    │
    ▼
build_start hooks(ctx, state)
    │
    ▼
ctx.bazel.build(...) / ctx.bazel.test(...)
    │
    ├─► per BES event ──► build_event hooks(ctx, state, event)
    │                        └─ artifact collection + upload
    │
    ▼
build_end hooks(ctx, state, exit_code)
    │
    ▼
return exit_code
```

Flags fed to Bazel come from:
- `BazelFragment.extra_startup_flags` / `extra_flags`
- `BazelFragment.startup_flags(...)` / `flags(...)` transforms (if set)
- `BazelFragment.build_event_sinks` → `--bes_backend` / `--bes_results_url`
- Command-line arguments passed by the user

### `delivery.axl`

Coordinates pushing build artifacts for a commit. It communicates with
`deliveryd`, a daemon running on the Workflows runner that tracks delivery state
and handles artifact signing.

```
delivery_start hook
    │
    ▼
deliveryd /record   ── register target labels + output SHAs
    │
    ▼
deliveryd /query    ── which targets are already delivered?
    │
    for each target:
    ├── skip if already delivered (unless --force)
    ├── bazel run --stamp <target>
    ├── deliveryd /deliver  ── sign + mark as delivered
    └── on failure: deliveryd /artifact/delete  ── allow retry
    │
    ▼
delivery_end hook
```

`deliveryd` listens on a Unix socket at the path in
`ASPECT_WORKFLOWS_DELIVERY_API_ENDPOINT` (format: `unix:///path/to/socket`).
All calls are plain HTTP/JSON over the socket (`lib/deliveryd.axl`).

---

## Artifact Upload

`lib/artifacts.axl` provides a single `upload_artifacts(ctx, files)` function
that dispatches to the correct CI platform backend.

| Platform | Detection | Method |
|----------|-----------|--------|
| GitHub Actions | `ACTIONS_RUNTIME_TOKEN` present | Twirp RPC: `CreateArtifact` → PUT to signed Azure Blob → `FinalizeArtifact` |
| Buildkite | `BUILDKITE_AGENT_ACCESS_TOKEN` present | `buildkite-agent artifact upload` CLI |
| GitLab CI | `CI_JOB_TOKEN` present | PUT to Generic Packages API |
| CircleCI | (no upload token; native integration) | Artifacts staged to `/workflows/testlogs` for `store_artifacts` step |

Files are collected during the build via `BazelFragment.build_event` by
inspecting BES `TestResult` and `NamedSetOfFiles` events, then archived with
`bsdtar` (`lib/tar.axl`) before upload.

---

## Build Metadata

`lib/build_metadata.axl` collects commit and CI context, exported to Bazel as
`--build_metadata=KEY=VALUE` flags. These appear in BES events consumed by
Bessie for analytics and dashboarding.

| Key | Source |
|-----|--------|
| `COMMIT_SHA` | `git show HEAD` → `GITHUB_SHA` / `BUILDKITE_COMMIT` / ... |
| `COMMIT_AUTHOR` | `git show HEAD` → CI env |
| `COMMIT_AUTHOR_EMAIL` | `git show HEAD` |
| `COMMIT_MESSAGE` | `git show HEAD` |
| `COMMIT_TIMESTAMP` | `git show HEAD` |
| `USER` | CI actor env var |
| `BRANCH` | `GITHUB_REF_NAME` / `BUILDKITE_BRANCH` / ... |
| `TAG` | `GITHUB_REF` (if tag ref) |
| `REPO_OWNER` | Parsed from remote URL |
| `REPO` | Parsed from remote URL |
| `VCS` | `github` / `gitlab` / `bitbucket` |
| `CI` | `github` / `buildkite` / `circleci` / `gitlab` |

---

## Health Checks and Cache Warming

`lib/health_check.axl` implements the warming and health check hooks registered
by `config/builtins.axl`.

**Warming flow:**
1. Poll until `ASPECT_WORKFLOWS_RUNNER_WARMING_COMPLETE_MARKER_FILE` exists.
2. Read `ASPECT_WORKFLOWS_RUNNER_WARMING_CACHE_VERSION_FILE`:
   - Success → print the cache version that was warmed.
   - Failure + first job on this instance → print a link to bootstrap logs
     (AWS CloudWatch or GCP Cloud Logging URL, based on `ASPECT_WORKFLOWS_RUNNER_CLOUD`).
3. Print the last health check summary from
   `ASPECT_WORKFLOWS_RUNNER_LAST_HEALTH_CHECK_FILE`.

**Health check flow:**
1. `ctx.bazel.health_check()` runs a lightweight Bazel invocation.
2. On failure: execute `/etc/aspect/workflows/bin/signal_instance_unhealthy`
   to mark the runner as unhealthy so Workflows can replace it.

---

## Extending Workflows Behavior from User Config

Any project can hook into Workflows behavior from its own `config/*.axl` files
by mutating the shared fragments.

**Example: add a custom Bazel flag on Workflows runners only**

```starlark
load("@aspect//fragments.axl", "BazelFragment")

def _configure(ctx):
    if ctx.std.env.var("ASPECT_WORKFLOWS_RUNNER"):
        ctx.fragments[BazelFragment].extra_flags = (
            ctx.fragments[BazelFragment].extra_flags +
            ["--config=workflows"]
        )

configure = config(impl = _configure)
```

**Example: notify Slack when delivery completes**

```starlark
load("@aspect//fragments.axl", "DeliveryFragment")

def _on_delivery_end():
    # post to Slack webhook ...

def _configure(ctx):
    ctx.fragments[DeliveryFragment].delivery_end = _on_delivery_end

configure = config(impl = _configure)
```

---

## Environment Variable Reference

| Variable | Used by | Purpose |
|----------|---------|---------|
| `ASPECT_WORKFLOWS_RUNNER` | detection | Non-empty when on a Workflows runner |
| `ASPECT_WORKFLOWS_RUNNER_STORAGE_PATH` | `Runner.storage_path` | Ephemeral storage mount |
| `ASPECT_WORKFLOWS_RUNNER_VERSION` | `Runner.product_version` | Workflows product version |
| `ASPECT_WORKFLOWS_RUNNER_INSTANCE_ID` | `Runner.instance_id` | Cloud instance ID |
| `ASPECT_WORKFLOWS_RUNNER_INSTANCE_NAME` | `Runner.instance_name` | Human-readable name |
| `ASPECT_WORKFLOWS_RUNNER_INSTANCE_TYPE` | `Runner.instance_type` | Cloud machine type |
| `ASPECT_WORKFLOWS_RUNNER_PREEMPTIBLE` | `Runner.preemptible` | Spot/preemptible instance |
| `ASPECT_WORKFLOWS_RUNNER_WARMING_ENABLED` | `Runner.warming_enabled` | Cache warming active |
| `ASPECT_WORKFLOWS_RUNNER_WARMING_COMPLETE_MARKER_FILE` | health_check | Path to warming-done marker |
| `ASPECT_WORKFLOWS_RUNNER_WARMING_CACHE_VERSION_FILE` | health_check | Path to warmed cache version |
| `ASPECT_WORKFLOWS_RUNNER_LAST_HEALTH_CHECK_FILE` | health_check | Path to last health check JSON |
| `ASPECT_WORKFLOWS_RUNNER_HAS_NVME_STORAGE` | `Runner.has_nvme_storage` | NVMe disk present |
| `ASPECT_WORKFLOWS_REMOTE_CACHE` | builtins.axl | gRPC remote cache endpoint |
| `ASPECT_WORKFLOWS_REMOTE_BYTESTREAM_URI_PREFIX` | builtins.axl | ByteStream URI prefix |
| `ASPECT_WORKFLOWS_BES_BACKEND` | builtins.axl | Bessie gRPC BES endpoint |
| `ASPECT_WORKFLOWS_BES_RESULTS_URL` | builtins.axl | BES results URL |
| `ASPECT_WORKFLOWS_DELIVERY_API_ENDPOINT` | delivery.axl | deliveryd Unix socket path |
| `ASPECT_WORKFLOWS_RUNNER_CLOUD` | health_check | Cloud provider (`aws` / `gcp`) |
