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
- wires the compact execution log through `bzl.exec_log` so `bazel_trait.exec_log_event` hooks see every action Bazel ran — see [Execution log hooks](DEVELOPMENT.md#execution-log-hooks),
- runs `hc_trait.health_check` hooks inside `setup_phase` so the runner-environment / health-check BK sections render before the build.

---

## build

[`build.axl`](build.axl) · `bazel build` over the user's target patterns.

```sh
aspect build                               # //... in the current package
aspect build //my/pkg/...
aspect build -- //... -//experimental/...   # exclusions need a `--` separator
aspect build --config=ci --keep_going //...          # Bazel flags typed directly
aspect build -c opt --jobs 8 //...                   # Bazel's own spellings work
```

Key flags:

| Flag                       | Default | Notes |
|----------------------------|---------|-------|
| `--bazel-flag`             | (none)  | Forwarded as-is. Repeatable. Rarely needed: type the Bazel flag directly. Reach for this when a Bazel flag collides with one of the task's own. |
| `--bazel-startup-flag`     | (none)  | The same, as a startup flag (server-restarting). Typing a startup flag before the task name does this for you. |
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
aspect gazelle --gazelle-target=//tools/gazelle:gazelle_from_source --config=pure_go
```

Why a single `-mode=diff` invocation rather than a check-then-fix sibling target: `with_check = True` only exists on `aspect_gazelle()` from `aspect_gazelle_prebuilt`; upstream `gazelle()` from rules_go has no equivalent. `-mode=diff` works on both.

Renderer: `gazelle_results`. Renders the gazelle target in the title (so `gazelle` and `gazelle_from_source` regression tasks render distinct titles), lists out-of-date BUILD files, and provides a `aspect gazelle` repro command.

---

## delivery

[`delivery.axl`](delivery.axl) · multi-phase delivery flow against a pluggable state backend.

```sh
aspect delivery //pkg/foo:release //pkg/bar:release
aspect delivery --mode=selective                # default: change-detect via state
aspect delivery --mode=always                   # always deliver (skip change detection)
aspect delivery --dry-run                       # preview without delivering
aspect delivery --dry-run=build                 # preview, but build the pending targets too
aspect delivery --force-target=//pkg/foo:release  # force one target through
aspect delivery --verify                        # run the tests that depend on the deliverables first; a failure blocks everything
aspect delivery --verify=skip                   # skip only the targets a failing test depends on
aspect delivery --verify=warn                   # deliver everything, warn about failures
```

Phase 1 builds the user's targets with the `hashsum_aspect` to compute action digests. Phase 2 queries the remote cache for those digests via `--experimental_remote_require_cached`. Phase 3 builds the pending targets and invokes their delivery entrypoints. Outcome buckets: `ok` / `skip` (already delivered) / `warn` / `fail` / `pending`.

`--verify[=block|skip|warn]` adds a phase between change detection and delivery: it queries `kind(".*_test rule", rdeps(//..., <targets to deliver>))` and runs the result as one `bazel test --keep_going`, streamed to the same build-event backends as the build, and the value says what a failing test does. `block` (a bare `--verify`) delivers nothing and exits with Bazel's code. `skip` asks Bazel which of the targets each failed test depends on (`deps(<test>) intersect <targets>`), marks exactly those `FAIL` with the failing tests named, delivers the rest, and still exits non-zero so CI goes red; they are not recorded as delivered, so the next run retries them. `warn` delivers everything and only warns. A target no test depends on always counts as verified. Only targets that would be delivered are verified, so a run where everything is already delivered runs no tests, and `--dry-run` skips it. A test is verified against a deliverable only if it depends on it, so a test that exercises the package or image (through `data` or `deps`) counts and an unrelated one does not. The test run also fills the delivery's Verified count: Aspect cloud counts the tests of builds with the same repository and commit as the delivery.

`--dry-run` is tri-state (`false` / `true` / `build`). A preview skips phase 3: it exists to put the pending targets' runfiles on disk for a dispatch that a dry-run never performs, so a preview neither stamps nor materializes release artifacts. `--dry-run=build` opts that build back in for previews that need the manifest's per-target on-disk paths or the `not runnable` check.

Renderer: `delivery_results`. The body shows counts-by-outcome, per-outcome tables (label / hash / context), failed deliveries open by default, plus the shared bazel detail body from phase 1.

State backend: the task gets its "already delivered" store from the `DeliveryState.connect` hook ([`delivery_state.axl`](delivery_state.axl)). Unset, it is `deliveryd`, the Workflows daemon on `ASPECT_WORKFLOWS_DELIVERY_API_ENDPOINT` ([`private/lib/deliveryd.axl`](private/lib/deliveryd.axl)). [`private/lib/redis_state.axl`](private/lib/redis_state.axl) talks to Redis directly with the same key scheme, on the generic client in [`redis.axl`](redis.axl) (URL parsing, TLS, AUTH, pipelining, one-shot reconnect; RESP2 codec in [`private/lib/resp.axl`](private/lib/resp.axl)), configured by `ASPECT_WORKFLOWS_DELIVERY_REDIS_ENDPOINT`, `_REDIS_CA_FILE`, `_REDIS_AUTH` and `_REDIS_LEGACY_TTL_SECONDS` (lifetime of the legacy `delivery-signature` keys it keeps writing for v1 deliveryd readers; default 7 days, `0` keeps them):

```python
load("@aspect//delivery_state.axl", "DeliveryState", "redis_state")

def config(ctx: ConfigContext):
    if DeliveryState in ctx.traits:
        ctx.traits[DeliveryState].connect = redis_state.connect
```

Aspect cloud as the state backend: `cloud_state.connect` ([`private/lib/delivery_cloud.axl`](private/lib/delivery_cloud.axl)) keeps the "already delivered" state in the Aspect cloud delivery backend instead of deliveryd or Redis. It needs Aspect credentials and nothing else; the organization is the credential's own. Select it the same way:

```python
load("@aspect//delivery_state.axl", "DeliveryState", "cloud_state")

def config(ctx: ConfigContext):
    if DeliveryState in ctx.traits:
        ctx.traits[DeliveryState].connect = cloud_state.connect
```

State is kept per repository and `--task:name[:--salt]` prefix: a target is skipped when its digest was already delivered under the same prefix, so every target is covered, including plain executables, and a rollback to an earlier digest still skips only while that digest is kept. A digest is stored only after a successful delivery, so a failed one is retried. A superseded digest is kept for 90 days. Without credentials the backend reports itself unavailable and the task stops at startup, like an unreachable deliveryd.

Aspect cloud records: with Aspect credentials (`aspect auth login` or `ASPECT_API_TOKEN`) every delivery attempt (and every skip) is also `POST`ed to Aspect cloud (`POST /delivery`, [`private/lib/delivery_record.axl`](private/lib/delivery_record.axl)), whichever state backend is in use. The record attaches to the phase-1 build when one was uploaded to a BES backend; without one only the delivered fact is kept. Without credentials nothing is recorded and nothing else changes.

- **Artifact.** Read from phase 1's BES `named_set_of_files` / `target_completed` events (the same invocation the record points at; phase 3 reports local `file://` paths and its stamped digests differ every run). The first default output named `*.deb`, `*.tar.gz` / `*.tgz` / `*.tar` (format `tar_gz`), an OCI image tarball, or an OCI layout's `index.json` wins; otherwise the output named like the target, or the only output, is the artifact with no `format`. `uri` is its `bytestream://` URI (or one built from `--remote_bytestream_uri_prefix` and the digest), `digest_hash` / `digest_size` come from `File.digest` / `File.length` (or the URI's `/blobs/<hash>/<size>`). No digest, no `artifact`.
- **Change detection.** Not part of recording: with `cloud_state` the same service answers `POST /delivery/query` for `{label: output digest}`, the digest the other backends key on.
- **What is recorded.** `exec` has the argv, the child's environment with only allowlisted names carrying values (`[redacted]` otherwise; `--record-env-allowlist` extends the built-in list), exit code, duration, `locus: local`, `where` (runner instance name, Buildkite agent, `RUNNER_NAME` or `HOSTNAME`), `retry_count: 0`, and `structured_output`, the last stdout line when it is a JSON object. `user_metadata` is the `DeliveryTrait.delivery_target` hook's `custom` dict; a string `version` / `tag` key in it also fills the record's `version` / `tag`. The console output is kept at its first and last 1 MiB per stream and sent in chunks, each request under 1 MiB.
- **Capture.** A serial delivery (`--max-parallelization=1`) used to inherit the console. While recording it writes to files that are tailed to the console as they grow, so the output still streams live, but the deliverable no longer sees a terminal.
- **Failure handling.** Recording never fails a delivery: a 401 / 403 / other error or a transport failure warns once and the run continues; a 409 / 503 (BES not ingested yet) is retried with backoff for up to 60 s after the deliveries finish, then dropped with a warning. `--dry-run` records nothing; `--record-deliveries=false` switches the whole thing off.

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
| [lib/delivery_record.axl](lib/delivery_record.axl) | `delivery_record.new`, `.artifact.pick`, `.apply_last_delivered`, `.capture.*`, `.log.*`, `Delivery` / `Artifact` / `Execution` records | delivery |

Each `*_results.axl` derives its `init_data()` from `bazel_results.init_data()` (so `process_event` can populate the full bazel state) and appends `SHARED_DETAILS_BODY_TEMPLATE` to its task-specific top section so the rendered details body has the same Targets / Build Metrics / Invocation / Workflows Runner / Workspace Status / Build Metadata / Options-parsed sections everywhere.
