# `aspect` built-in tasks

Aspect-CLI ships with six built-in tasks that drive Bazel for the most common CI workflows. Each task pairs a Bazel command with cross-cutting integration — GitHub status checks, Buildkite annotations, artifact upload, runner health checks — so the underlying Bazel invocation surfaces consistently across every supported CI host.

> Looking for *how* to add a task or extend an existing one? See [`DEVELOPMENT.md`](DEVELOPMENT.md).

## Quick reference

| Task            | Verb / target            | Primary command surface                                     | `task_update.kind`   |
|-----------------|--------------------------|-------------------------------------------------------------|----------------------|
| [build](#build) | `bazel build`            | `aspect build [-- //... -//experimental/...]`               | `bazel_results`      |
| [test](#test)   | `bazel test`             | `aspect test [-- //... -//experimental/...]`                | `bazel_results`      |
| [lint](#lint)   | `bazel build` + lint aspects | `aspect lint --aspect=//tools/lint:linters.bzl%shellcheck`  | `lint_results`       |
| [format](#format) | `bazel run` of a `format_multirun` | `aspect format [--scope=changed\|all]`                       | `format_results`     |
| [gazelle](#gazelle) | `bazel run` of a `gazelle()` / `aspect_gazelle()` target | `aspect gazelle [--check]`                                   | `gazelle_results`    |
| [delivery](#delivery) | Multi-phase delivery flow | `aspect delivery //pkg/foo:release //pkg/bar:release`        | `delivery_results`   |

Every task:

- emits **`task_update`** events through its lifecycle — the *first* (the `🔧 Setup` phase mark from `setup_phase`) drives surface init (GitHub check run + BK annotation) and carries the `subject`; middle ones live-render BES progress; the *last* (`final=True`) concludes the surfaces. There is a single lifecycle hook; the first and last updates carry the start/complete intent,
- captures the gRPC sink's invocation UUID into `results["sink_invocation_id"]` immediately after `ctx.bazel.build(...)` returns so the **Aspect Workflows** link surfaces in the live annotation rather than only on completion,
- runs `bazel_trait.build_start` / `bazel_trait.build_event` / `bazel_trait.build_end` hooks so features (artifact upload, BK section markers) fire,
- runs `hc_trait.health_check` hooks inside `setup_phase` so the runner-environment / health-check BK sections render before the build.

---

## build

[`build.axl`](build.axl) · `bazel build` over the user's target patterns.

```sh
aspect build                               # //... in the current package
aspect build //my/pkg/...
aspect build -- //... -//experimental/...   # exclusions need a `--` separator
aspect build --bazel-flag=--config=ci --bazel-flag=--keep_going //...
```

Key flags:

| Flag                       | Default | Notes |
|----------------------------|---------|-------|
| `--bazel-flag`             | (none)  | Forwarded as-is. Repeatable. |
| `--bazel-startup-flag`     | (none)  | Forwarded as a startup flag (server-restarting). |
| `--bes-backend`            | (none)  | `--bes_backend=<value>`. Repeatable. |
| `--bes-header`             | (none)  | `--bes_header=<value>`. Repeatable. |
| `--cancel`                 | `false` | Cancels any running invocation first. |
| `--bazel-output-base`      | (none)  | Pin the Bazel server instance. |
| `--target-pattern-file`    | (none)  | Newline-separated target patterns (mirrors Bazel `--target_pattern_file`). Cannot be combined with command-line patterns. |

Produces:

- **GHSC / BK annotation** — `bazel_results` summary (Targets / Build Metrics / Invocation / Workflows Runner / Workspace Status / Build Metadata / Options parsed).
- **Artifact uploads** (when `ArtifactUpload` is enabled): profile, BEP, execlog.
- **Reproducer command** under failures — copy-pasteable `aspect build //failed:tgt` line.

---

## test

[`test.axl`](test.axl) · `bazel test` over the user's target patterns.

Same flag surface as [`build`](#build), plus:

- `--coverage` — collect code coverage (`--collect_code_coverage --combined_report=lcov`). The merged LCOV report lands at `$(bazel info output_path)/_coverage/_coverage_report.dat`.
- `--coverage-report=PATH` — copy the merged LCOV report to `PATH` after a `--coverage` run.
- `--coverage-tool=BIN` + `--coverage-tool-arg=ARG` (repeatable) — run an external tool against the merged LCOV report (e.g. `genhtml`, `codecov`). `{report}` (or `{lcov}`) in any arg is replaced with the absolute report path; absent any placeholder, the path is appended as the last positional arg. The tool runs regardless of the test exit code, and tool failures surface as warnings (so uploaders can publish partial coverage).

  ```sh
  # genhtml: report path as positional
  aspect test --coverage \
    --coverage-tool=genhtml --coverage-tool-arg=-o --coverage-tool-arg=coverage-html

  # codecov: report path behind a flag
  aspect test --coverage \
    --coverage-tool=codecov --coverage-tool-arg=-f --coverage-tool-arg={report}
  ```

The renderer adds:

- **Test summary** rows in the BK / GHSC body: passed / failed / flaky / cached / timed-out, with per-test logs linked from the artifacts row when uploads are enabled.
- **Reproducer command** unifying build failures + failed tests (one copy-paste reruns everything that broke).

`task_update.kind` is `bazel_results`; build and test share a renderer because the BES event shape is the same modulo the extra test-specific events.

---

## lint

[`lint.axl`](lint.axl) · `bazel build --aspects=…` + SARIF processing.

```sh
aspect lint --aspect=//tools/lint:linters.bzl%eslint
aspect lint --aspect=//:linters.bzl%shellcheck -- //... -//excluded/...
aspect lint --strategy=hold-the-line     # only fail on errors in changed files (default)
aspect lint --strategy=hard              # fail on any error
aspect lint --strategy=soft              # surface diagnostics, never fail
aspect lint --fix                        # apply rules_lint patches in-place
```

Required: at least one `--aspect=//path:linters.bzl%name`. Repeatable to run multiple linters in one invocation.

Strategies (`--strategy=`):

| Strategy        | Fails on                                                                              |
|-----------------|---------------------------------------------------------------------------------------|
| `hold-the-line` *(default)* | error-severity diagnostics anchored on **changed lines** only             |
| `hold-the-file` | error-severity diagnostics in any **touched file** (every finding on that file)       |
| `hard`          | any error or linter-process failure                                                   |
| `soft`          | nothing (diagnostics still rendered)                                                  |

Diagnostic destination — `LintTrait.findings_destination`:

- `auto` *(default)* — split by surface affordance: on a PR, fix-bearing findings post as PR review comments (suggestion block renders inline) and non-fix findings post as check-run annotations; off a PR, every finding posts as an annotation (no comments surface available).
- `comments` — every finding posts as a PR review comment. Requires a PR.
- `annotations` — every finding posts as a check-run annotation. Suggestion blocks are dropped (the check-runs API has no patch primitive).
- `both` — every finding posts to BOTH surfaces. Reviewers see them inline in Files Changed *and* in the Conversation thread. Use when annotation visibility is the dominant constraint (some review workflows hide annotations).

Renderer: `lint_results`. Body has by-severity counts, by-tool counts, per-severity tables, plus the shared bazel detail body (Targets / Build Metrics / etc.) at the bottom.

---

## format

[`format.axl`](format.axl) · build a formatter target, run it on the changed file list, diff before-and-after.

```sh
aspect format                                                        # changed scope (default)
aspect format --scope=all                                            # whole tree
aspect format --formatter-target=@buildifier_prebuilt//buildifier    # buildifier-only
aspect format --include-pattern='**/*.bzl'                           # only bzl files
aspect format --exclude-pattern='vendor/**'                          # skip vendor
aspect format --severity=warn                                        # warn but don't fail CI
aspect format --upload-format-diff                                   # upload `format.patch`
```

Verdict comes from a `git diff` between the pre-format and post-format working tree (after applying `--include-pattern` and `--exclude-pattern`). Non-empty diff + `--severity=fail` → exit 1; `--severity=warn` → exit 0 with status=warning; `--severity=info` → exit 0 with status=passed. The formatter binary's own non-zero exit fails the task regardless of `--severity`.

Renderer: `format_results`. The check-run / annotation summary shows the formatter-target label in the title (e.g. `Format //tools/format · 3 files need formatting`); the body lists affected files with a `aspect format` repro command and (when `--upload-format-diff`) a download link to the captured patch.

---

## gazelle

[`gazelle.axl`](gazelle.axl) · build a `gazelle()` / `aspect_gazelle()` target, run it once with `-mode=diff` to capture the patch, optionally apply it via `git apply -p0`.

```sh
aspect gazelle                                       # apply changes via git apply
aspect gazelle --check                               # CI mode: report only, don't apply
aspect gazelle --check tools/go services/api         # limit to specific subtrees
aspect gazelle --gazelle-flag=-progress              # forward flags to gazelle
aspect gazelle --gazelle-command=fix                 # use 'fix' subcommand
aspect gazelle --gazelle-target=//tools/gazelle:gazelle_from_source --bazel-flag=--config=pure_go
```

Why a single `-mode=diff` invocation rather than a check-then-fix sibling target: `with_check = True` only exists on `aspect_gazelle()` from `aspect_gazelle_prebuilt`; upstream `gazelle()` from rules_go has no equivalent. `-mode=diff` works on both.

Renderer: `gazelle_results`. Renders the gazelle target in the title (so `gazelle` and `gazelle_from_source` regression tasks render distinct titles), lists out-of-date BUILD files, and provides a `aspect gazelle` repro command.

---

## delivery

[`delivery.axl`](delivery.axl) · multi-phase delivery flow against a `deliveryd` backend.

```sh
aspect delivery //pkg/foo:release //pkg/bar:release
aspect delivery --mode=selective                # default: change-detect via state
aspect delivery --mode=always                   # always deliver (skip change detection)
aspect delivery --dry-run                       # preview without delivering
aspect delivery --force-target=//pkg/foo:release  # force one target through
```

Phase 1 builds the user's targets with the `hashsum_aspect` to compute action digests. Phase 2 queries the remote cache for those digests via `--experimental_remote_require_cached`. Phase 3 invokes per-target delivery entrypoints. Outcome buckets: `ok` / `skip` (already delivered) / `warn` / `fail` / `pending`.

Renderer: `delivery_results`. The body shows counts-by-outcome, per-outcome tables (label / hash / context), failed deliveries open by default, plus the shared bazel detail body from phase 1.

---

## Cross-cutting features

| Feature                  | File                                                                       | Activation                                              | What it does |
|--------------------------|----------------------------------------------------------------------------|---------------------------------------------------------|--------------|
| `Workflows`              | [feature/workflows.axl](feature/workflows.axl)                             | Opt-in via config; only fully active on Aspect Workflows runners | Wires BES forwarding, build metadata, runner health checks, `--color=yes`, CI-host quirks (GitHub runner-tracking suppression, Buildkite section markers). |
| `GithubStatusChecks`     | [feature/github_status_checks.axl](feature/github_status_checks.axl)       | `enabled = True` (default); skips silently outside CI / non-github.com host | Creates / updates a GitHub check run via the Aspect GitHub App token. |
| `GithubStatusComments`   | [feature/github_status_comments.axl](feature/github_status_comments.axl)   | `enabled = True` (default); GitHub Actions + PR only    | PR-level *aggregated* sticky comment that rolls every sibling task into one body. Polled and PATCHed in place. |
| `GithubLintComments`     | [feature/github_lint_comments.axl](feature/github_lint_comments.axl)       | Opt-in via `LintTrait.findings_destination` ∋ `comments` | Posts SARIF findings as PR review comments (channel 2) and rules_lint patches as suggestion-block comments (channel 3). |
| `GitlabLintComments`     | [feature/gitlab_lint_comments.axl](feature/gitlab_lint_comments.axl)       | `enabled = True` (default); fires on `aspect lint` task updates when a GitLab MR context is detected (`CI_MERGE_REQUEST_IID` on GitLab CI, or the `ASPECT_GITLAB_*` overrides on Aspect Workflows runners on other hosts) | Posts `aspect lint` findings as inline GitLab MR diff discussions, with one-click suggestion blocks where a fix patch is available — the GitLab counterpart to `GithubLintComments`. Auth via the Aspect GitLab App token (`api` scope), same flow as `GitlabStatusComments`. |
| `BuildkiteAnnotations`   | [feature/buildkite_annotations.axl](feature/buildkite_annotations.axl)     | `enabled = True` (default); requires `BUILDKITE` env    | Posts `buildkite-agent annotate --scope=job` annotations with a leading task pill. |
| `ArtifactUpload`         | [feature/artifacts.axl](feature/artifacts.axl)                             | `args.upload_*` flags per-task                          | Uploads testlogs / build-failure logs / profile / BEP / execlog to the host CI; populates `ArtifactsTrait`. |
| `Telemetry`              | [feature/telemetry.axl](feature/telemetry.axl)                             | `ctx.telemetry.exporters.add(...)` in config            | OTLP traces / logs / metrics export. |
| `Tips`                   | [feature/tips.axl](feature/tips.axl)                                       | `enabled = True` (default)                              | Collects per-task tips and surfaces them on the terminal, check-runs, and PR comments. Disable or silence ids via `ctx.features[Tips]` in config. |

`GithubStatusChecks` and `BuildkiteAnnotations` delegate per-kind rendering through [`lib/check_dispatch.axl`](lib/check_dispatch.axl) so adding a new task kind is a single dispatch-table entry rather than an N×2 update. `GithubStatusComments` aggregates *across* tasks (one PR comment per run); `GithubLintComments` posts at the diagnostic-anchor level inside the PR's Files Changed view.

### GitLab lint comments

`GitlabLintComments` is the GitLab counterpart to `GithubLintComments`: it posts `aspect lint` findings as inline GitLab Merge Request diff discussions, rendering a one-click suggestion block on any finding that carries a `rules_lint` fix patch. It fires on `aspect lint` task updates whenever a GitLab MR context is detected — the same detection the other GitLab features use (`CI_MERGE_REQUEST_IID` on GitLab CI, or the `ASPECT_GITLAB_*` overrides for Aspect Workflows runners on other hosts) — and authenticates with the Aspect GitLab App token (requesting the `api` scope) through the same token flow as `GitlabStatusComments`.

Config knobs:

| Knob | Default | Notes |
|------|---------|-------|
| `max_pr_comments` | `25` | Maximum MR discussions posted per run. Errors are prioritized over warnings, then notes; excess findings are dropped from the MR but still drive the lint task's exit code. Set to `0` to disable posting while keeping exit-code behavior intact. |
| `findings_destination` | `auto` | Shared `LintTrait` knob (`auto` / `comments` / `annotations` / `both`) controlling whether findings post to inline discussions, annotations, or both — see [`lint`](#lint) for the full per-value breakdown. |
| `only_annotate_changed_regions` | — | Shared knob; findings outside the displayed MR diff hunk are not posted. |

Behavior notes:

- **De-duplicates across reruns.** Re-running the lint task replaces stale annotations at the same position rather than stacking duplicate discussions.
- **Auto-resolves on a clean pass.** On a genuinely clean lint pass (no findings), prior Aspect-owned lint threads are auto-resolved — a GitLab-only affordance the GitHub path lacks (GitHub PR review comments have no native resolve).

## Per-kind result libraries

| Library                                | Public API                                                                        | Used by |
|----------------------------------------|-----------------------------------------------------------------------------------|---------|
| [lib/bazel_results.axl](lib/bazel_results.axl) | `init_data`, `process_event`, `render_check_output`, `compute_reproducer_command`, `resolve_aspect_url`, `build_summary_data`, `build_details_data`, `SHARED_DETAILS_BODY_TEMPLATE` | build, test (and as the base for the others) |
| [lib/lint_results.axl](lib/lint_results.axl) | `init_data`, `accumulate`, `render_check_output`, `compute_annotations`, `chunk_annotations` | lint |
| [lib/format_results.axl](lib/format_results.axl) | `init_data`, `render_check_output`, `format_summary_title` | format |
| [lib/gazelle_results.axl](lib/gazelle_results.axl) | `init_data`, `render_check_output`, `gazelle_summary_title` | gazelle |
| [lib/delivery_results.axl](lib/delivery_results.axl) | `init_data`, `add_result`, `render_check_output`, `delivery_summary_title` | delivery |

Each `*_results.axl` derives its `init_data()` from `bazel_results.init_data()` (so `process_event` can populate the full bazel state) and appends `SHARED_DETAILS_BODY_TEMPLATE` to its task-specific top section so the rendered details body has the same Targets / Build Metrics / Invocation / Workflows Runner / Workspace Status / Build Metadata / Options-parsed sections everywhere.
