# Rosetta Feature Migration Status

Tracking what existed in Rosetta (TypeScript) and where each feature ended up in the new architecture.

## Architecture Changes

The new system no longer wraps the CI configuration file. Instead, the CI configuration calls into the Aspect CLI directly. This eliminates the need for pipeline generation, condition expressions, multi-workspace orchestration, and several system tasks that only existed to support the wrapper model.

## Task Types

| Task | Status | Notes |
|------|--------|-------|
| build | Implemented | `@aspect//build.axl` builtin + axel-f config hooks |
| test | Implemented | `@aspect//test.axl` builtin + axel-f config hooks |
| lint | Implemented | Via `@aspect_rules_lint` package + axel-f GitHub integration |
| format | Implemented | Via `@aspect_rules_lint` package |
| delivery | Implemented | `axel-f/tasks/delivery.axl` with deliveryd integration |
| configure | Not yet implemented | Third-party package |
| gazelle | Not yet implemented | Third-party package |
| buildifier | Not yet implemented | Third-party package |
| warming | Not yet implemented | `bazel build --nobuild` to pre-warm cache on dedicated queue |
| checkout / branch freshness | Not yet implemented | Stale branch detection, rebase/merge, annotations |
| bazel_health_probe | Removed | No longer needed |
| finalization | Removed | No longer needed |
| delivery_manifest | Removed | Architectural change — delivery no longer uses manifests |
| noop | Removed | Was only used for testing |

## Major Features

### Implemented in axel-f

| Feature | Location | Notes |
|---------|----------|-------|
| Platform config reading | `lib/platform.axl` | Reads `/etc/aspect/workflows/platform/` files |
| Bazelrc flag generation | `lib/platform.axl` | Dynamic flags based on Bazel version, host, platform config |
| BES sink configuration | `config/builtins.axl` | Reads bessie_endpoint, configures gRPC sink |
| GitHub check runs | `lib/github.axl` | Create/update check runs with annotations |
| GitHub PR reviews | `lib/github.axl` | Review comments, code suggestions |
| SARIF translation | `lib/sarif.axl` | SARIF lint output to GitHub review comments |
| Lint hold-the-line | `config/lint.axl` | GitHub-aware lint strategy with changed file filtering |
| CI config migration | `tasks/migrate.axl` | Generates GitHub Actions / Buildkite configs from old workflows.yaml |
| Bazel exit codes | `@aspect//bazel.axl` | Exit code constants and default retry predicate |
| Build metadata | `config/builtins.axl` | CI info attached to bazel invocations |
| Deliveryd integration | `lib/deliveryd.axl` | Unix socket HTTP client for delivery daemon |

### Moved to Aspect CLI (Rust)

| Feature | Notes |
|---------|-------|
| OpenTelemetry tracing & metrics | Built into the CLI runtime |
| Bazel version detection | Built into the CLI runtime |
| Shell execution with tracing | Built into the CLI runtime |

### Removed — architectural changes

| Feature | Reason |
|---------|--------|
| Pipeline generation (steps command) | CI config now calls the CLI directly, not the other way around |
| Condition expressions (`when:` clauses) | No longer wrapping CI config; CI platform handles conditional execution |
| Multi-workspace orchestration | CI config handles workspace matrix; CLI runs in a single workspace |
| Marvin result reporting | Replaced with direct GitHub API calls from axel-f |
| Configuration validation (config command) | No longer have a workflows.yaml to validate |
| Event bus (RxJS pub/sub) | Not needed; task lifecycle is simpler without the wrapper model |
| CI host abstraction (Host interface) | CI platform is known at config time; no runtime detection needed |
| Buildkite annotations (buildkite-agent annotate) | CI platform handles its own annotations |
| Task hooks (before_task / after_task) | CI platform handles pre/post steps natively |
| Bazel server log archiving on crash | Can be handled by CI platform artifact upload |
| Debug assistance mode (execution logs, gRPC logs) | Users enable these via bazel flags directly |

## Not Yet Implemented

| Feature | Priority | Description |
|---------|----------|-------------|
| Warming task | TBD | Run `bazel build --nobuild` on targets from build/test tasks to warm the remote cache on a dedicated queue |
| Checkout / branch freshness | TBD | Detect stale branches, run rebase/merge, post annotations with fix commands |
