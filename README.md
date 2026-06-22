# Aspect CLI

`aspect` is a free, open-source, programmable task runner built on top of Bazel. It replaces the pile of bespoke shell scripts and CI YAML that every Bazel monorepo eventually grows — format pre-submits, lint enforcement, BUILD-file generation, release-tag delivery — with a single command-line tool that behaves identically on a developer laptop and in CI.

```text
$ aspect test //...
→ 🎬 Running `test` task

→ 🔧 Setup · Running setup

→ 🧪 Test · Spawning bazel test
INFO: Bazel 9.0.1
INFO: Analyzed 147 targets (0 packages loaded, 0 targets configured).
INFO: Found 122 targets and 25 test targets...
INFO: Elapsed time: 0.866s, Critical Path: 0.07s
INFO: 1 process: 31 action cache hit, 1 internal.
INFO: Build completed successfully, 1 total action

Executed 0 out of 25 tests: 25 tests pass.
INFO: Build Event Protocol files produced successfully.

→ ✅ Passed `test` task in 1.2s · Tests passed (cached)
    🔧 Setup   2ms  Prepare the task environment
    🧪 Test   1.2s  Run bazel tests
```

Every `aspect <task>` ends with that per-phase breakdown, so the slow part of a CI step is always called out by name. The same content gets posted back to your PR as Buildkite annotations, GitHub Status Checks, and a PR task summary comment (see [examples below](#see-it-in-action)). [The CLI overview](https://aspect.build/docs/cli/overview#what-youll-see) shows `aspect format`, `aspect buildifier`, and `aspect lint` runs too — including the hold-the-line output (linter surfaces findings in unmodified files; the task still passes because no *new* violations were introduced).

## Configure and extend in AXL

[AXL, the Aspect Extension Language](https://aspect.build/docs/cli/overview#aspect-extension-language), is how you configure built-in tasks (in `.aspect/config.axl`) and add your own (as `.aspect/*.axl` files). It's a [typed Starlark](https://github.com/facebook/starlark-rust/blob/main/docs/types.md) dialect evaluated by the [Rust Starlark](https://github.com/facebook/starlark-rust) interpreter built by the [Buck2](https://buck2.build/) team, so `.axl` files catch type errors at parse time and parse fast even on huge repos.

Here's an example `.aspect/config.axl` that exercises several built-ins (for a full live config running in CI, see [`aspect-build/bazel-examples`](https://github.com/aspect-build/bazel-examples/blob/main/.aspect/config.axl)):

```python
"""Aspect CLI configuration."""

load("@aspect//feature/artifacts.axl", "ArtifactUpload")
load("@aspect//format.axl", "format")
load("@aspect//traits.axl", "BazelTrait")

buildifier = format.alias(
    defaults = {
        "formatter_target": "@buildifier_prebuilt//buildifier",
        "formatter_args_for_tree_walk": ["-r", "."],
        "run_in": "cwd",
        "include_patterns": [
            "**/BUILD",
            "**/BUILD.bazel",
            "**/MODULE.bazel",
            "**/*.MODULE.bazel",
            "**/WORKSPACE",
            "**/WORKSPACE.bazel",
            "**/*.axl",
            "**/*.bzl",
            "**/*.star",
        ],
    },
    summary = "Format Starlark files using buildifier.",
)

def config(ctx: ConfigContext):
    # Set --config=ci on all bazel commands on all CI environments.
    if bool(ctx.std.env.var("CI")):
        ctx.traits[BazelTrait].extra_flags.extend([
            "--config=ci",
        ])

    # Register the buildifier alias as a CLI command.
    ctx.tasks.add(buildifier)

    # Lint aspects — required by the built-in `aspect lint` task. Same set
    # locally as on CI; CI invocations don't need to repeat the --aspect flags.
    ctx.tasks["lint"].args.aspects = [
        "//tools/lint:linters.bzl%buf",
        "//tools/lint:linters.bzl%checkstyle",
        "//tools/lint:linters.bzl%clippy",
        "//tools/lint:linters.bzl%eslint",
        "//tools/lint:linters.bzl%keep_sorted",
        "//tools/lint:linters.bzl%pmd",
        "//tools/lint:linters.bzl%ruff",
        "//tools/lint:linters.bzl%shellcheck",
    ]

    # Delivery.
    ctx.tasks["delivery"].args.query = "kind(\"oci_push rule\", //...)"
    ctx.tasks["delivery"].args.bazel_flags = [
        "--config=release",
    ]

    # Enable artifact uploads for testlogs, profile, and BEP.
    # upload_test_logs="failed" — the logs from passing tests are noise;
    # failing/flaky tests' logs are the ones anyone would actually open.
    ctx.features[ArtifactUpload].args.upload_test_logs = "failed"
    ctx.features[ArtifactUpload].args.upload_profile = True
    ctx.features[ArtifactUpload].args.upload_bep = True
```

## Why another tool on top of Bazel?

`bazel` is a build system, not a developer-workflow tool. Every Bazel monorepo eventually grows the same scaffolding by hand — format pre-submits, lint enforcement, BUILD-file generation, release-tag delivery — written in shell or YAML, copy-pasted across teams, behaving differently in CI than on a laptop.

`aspect` replaces that scaffolding. It's a programmable task-runner layer on top of `bazel`: built-in tasks you tune in `.aspect/config.axl`, custom tasks you add as `.aspect/*.axl` files, and a clean fallthrough to raw `bazel` for anything `aspect` doesn't wrap. You're not switching build systems — you're extending the one you already have.

**What's in the box:**

- **Same command, every environment.** `aspect lint` on a laptop does the same thing as in a CI pipeline. Cuts an entire class of "works on my machine but not in CI" bugs.
- **The CI scaffolding every Bazel repo eventually re-invents** — built in: hold-the-line lint (fails only on violations *you* introduced), selective delivery (re-deploys only services whose Bazel outputs actually changed), smart changed-file detection, bounded retry on transient Bazel errors, native artifact upload, and per-step status checks on GitHub, Buildkite, GitLab, and CircleCI.
- **Custom CLI commands in ~10 lines of Starlark.** Drop a `.axl` file into `.aspect/` and `aspect <name>` is a real CLI command (see below). Tasks can shell out, read/write files, query the build graph, and subscribe to Bazel's Build Event Stream and Compact Execution Log.
- **Per-repo version pin.** `.aspect/version.axl` pins the CLI version; the launcher fetches the matching binary on first invocation, so local and CI stay in sync.
- **Standalone or with Aspect Workflows.** The CLI works on its own with any Bazel workspace. Pair it with [Aspect Workflows](https://aspect.build/docs/aspect-workflows) — Aspect's managed Bazel-CI runners, deployed in your AWS or GCP account or hosted by Aspect — for sub-minute cached builds, 2–3× faster CI, 40–80% cloud-compute savings, plus a Web UI, remote cache, and remote build execution.

## Install

```shell
curl -fsSL https://install.aspect.build | bash
```

macOS and Linux. Apache-2.0. No Aspect account required.

The [10-minute Quickstart](https://aspect.build/docs/quickstart) walks from install to writing a custom task. Full docs: [aspect.build/docs/cli](https://aspect.build/docs/cli/overview).

## Built-in tasks

Three tasks work in any Bazel workspace with no extra setup — they're `bazel build` / `bazel test` / `bazel run` plus the retry, BES streaming, artifact upload, status checks, and CI-platform reporting described above:

| Task | What it does |
|---|---|
| [`aspect build`](https://aspect.build/docs/cli/tasks/build_test) | Build Bazel targets |
| [`aspect test`](https://aspect.build/docs/cli/tasks/build_test) | Run Bazel tests, with optional LCOV coverage |
| [`aspect run`](https://aspect.build/docs/cli/tasks/run) | Build and run a binary target |

The remaining built-ins need both **Bazel-graph wiring** (a tool dependency in `MODULE.bazel`, plus a BUILD target or rule wired up to it) and **AXL wiring** (programmable configuration in `.aspect/config.axl`) to point the task at your repo's setup. Each task page walks through both:

| Task | What it does | What it needs |
|---|---|---|
| [`aspect format`](https://aspect.build/docs/cli/tasks/format) | Format files changed in the PR | A `//tools:format` BUILD target (the task's default) that wraps a formatter binary — typically via [`aspect_rules_lint`](https://registry.bazel.build/modules/aspect_rules_lint) or [`buildifier_prebuilt`](https://registry.bazel.build/modules/buildifier_prebuilt), plus the matching `bazel_dep`. Override the target path with `formatter_target` in `.aspect/config.axl` if you put it somewhere else. |
| [`aspect lint`](https://aspect.build/docs/cli/tasks/lint) | Run linters with hold-the-line strategy | [`aspect_rules_lint`](https://registry.bazel.build/modules/aspect_rules_lint) plus the linter rules of choice (eslint, ruff, golangci-lint, …); lint aspects declared in `.aspect/config.axl` |
| [`aspect gazelle`](https://aspect.build/docs/cli/tasks/gazelle) | Generate and sync BUILD files | A `//tools:gazelle` BUILD target (the task's default) that wraps a Gazelle binary — typically via [`gazelle`](https://registry.bazel.build/modules/gazelle) or [`aspect_gazelle_prebuilt`](https://registry.bazel.build/modules/aspect_gazelle_prebuilt) (for Starlark-defined extensions), plus the matching `bazel_dep`. Override the target path with `gazelle_target` in `.aspect/config.axl` if you put it somewhere else. |
| [`aspect delivery`](https://aspect.build/docs/cli/tasks/delivery) | Deliver only targets whose Bazel-built outputs changed | BUILD targets that implement delivery (e.g. `oci_push`, `helm_push`, custom `bazel_run`-able scripts); a `--query` (or `config.axl` equivalent) selecting which targets are deliverables |

`aspect help` lists every task available in your repo (built-ins plus any custom ones you've added).

## Custom tasks

Drop a `.axl` file into `.aspect/`, define a task, and `aspect <name>` is a CLI command:

```python
# .aspect/codegen.axl
def _impl(ctx: TaskContext) -> int:
    return ctx.bazel.build(*ctx.args.targets).wait().code

codegen = task(
    summary = "Run the code generator.",
    implementation = _impl,
    args = {
        "targets": args.positional(default = ["//gen/..."]),
    },
)
```

```shell
aspect codegen //gen/services/...
```

Tasks can shell out to any subprocess, read and write files, query the build graph, subscribe to Bazel's Build Event Stream, and declare typed arguments. [How to run and define tasks](https://aspect.build/docs/cli/guides/basic) covers the full walkthrough. The [AXL reference](https://aspect.build/docs/axl/types) documents every type and built-in.

## Running in CI

The same `aspect <task>` command you run locally works identically in CI. [Running tasks in CI](https://aspect.build/docs/cli/tasks-ci) has ready-to-paste YAML for GitHub Actions, Buildkite, GitLab CI, and CircleCI — for both provider-hosted runners and Aspect Workflows CI runners.

On GitHub Actions specifically, [`aspect-build/setup-aspect`](https://github.com/aspect-build/setup-aspect) handles the install, wires up GHA caching, and exchanges your `ASPECT_API_TOKEN` for a short-lived JWT — in one action step.

## See it in action

The [`aspect-build/bazel-examples`](https://github.com/aspect-build/bazel-examples) repo runs `aspect <task>` pipelines on every commit across all four supported CI providers. Click through to inspect a current build:

| CI provider | Live pipeline |
|---|---|
| GitHub Actions | [Actions tab](https://github.com/aspect-build/bazel-examples/actions?query=branch%3Amain) |
| Buildkite | [Recent builds](https://buildkite.com/aspect-build/bazel-examples/builds?branch=main) |
| GitLab CI/CD | [Pipelines](https://gitlab.com/aspect-build/bazel-examples/-/pipelines?scope=all&ref=main) |
| CircleCI | [Pipeline runs](https://app.circleci.com/pipelines/github/aspect-build/bazel-examples?branch=main) |

`aspect <task>` posts task results to three surfaces:

- **PR task summary comment** — a single comment summarising every task in the pipeline.
- **GitHub Status Checks** — one check per `aspect <task>` invocation, named by `--task-name`.
- **Buildkite annotations** _(when running on Buildkite)_ — one annotation per `aspect <task>` invocation, rendered at the top of the build page.

Live links to each, from real runs of this repo's own CI: [What `aspect <task>` reports back](https://aspect.build/docs/cli/tasks-ci#what-aspect-%3Ctask%3E-reports-back).

## Comparison to older versions

> [!NOTE]
> This is a Rust rewrite, superseding the legacy Go version published as version 2025.41 and earlier.
> The older implementation is now in maintenance mode at [aspect-build/aspect-cli-legacy](https://github.com/aspect-build/aspect-cli-legacy).

Versions before 2025.42 differed in some notable ways:

- Older versions shadowed the `bazel` command in the recommended installation, using homebrew or `bazeliskrc` to override `bazel`. Now `aspect` works alongside `bazel`.
- The plugin system used a gRPC client/server protocol. Now `aspect` uses Starlark via the Aspect Extension Language (AXL).
- Older versions included a fully pre-compiled Gazelle binary along with some Gazelle extensions, using the `configure` command. This has moved to a standalone repo: [aspect-build/aspect-gazelle](https://github.com/aspect-build/aspect-gazelle).

## Community and support

- Documentation: [aspect.build/docs/cli](https://aspect.build/docs/cli/overview)
- Quickstart: [aspect.build/docs/quickstart](https://aspect.build/docs/quickstart)
- Slack: [slack.aspect.build](https://slack.aspect.build/)
- Issues and discussions: this repo

## License

[Apache License 2.0](./LICENSE).
