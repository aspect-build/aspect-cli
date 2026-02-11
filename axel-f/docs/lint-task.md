# Lint Task

This document describes how the lint task works within the Rosetta workflow system. It is aimed at internal engineers who maintain or extend the task.

## Overview

The lint task wraps `aspect lint` — a Bazel-native linting command — and adds CI-aware behavior on top of it. Its primary job is to:

1. Run `aspect lint` against a set of Bazel targets.
2. Parse the structured lint output (a JSON diagnostics file produced from SARIF).
3. Decide which diagnostics are relevant to the current change.
4. Determine whether the task should pass or fail.
5. Surface results as GitHub check annotations and PR review suggestions.

## Configuration

The lint task is configured through a schema that extends the base Bazel task configuration. The lint-specific fields are:

| Field | Type | Default | Description |
|---|---|---|---|
| `targets` | `string[]` | `['//...']` | Bazel target patterns to lint. |
| `failure_strategy` | `'soft' \| 'hard' \| 'hold_the_line'` | `'hold_the_line'` | Controls when a lint failure causes the task to go red. See [Failure Strategies](#failure-strategies). |
| `only_annotate_changed_regions` | `boolean` | `true` | When true, GitHub annotations are scoped to files changed in the current PR. When false, all lint issues are annotated. |
| `icon` | `string` | `'broom'` | Emoji used in the task label. |

## Failure Strategies

### The problem

Large codebases accumulate lint violations over time. Turning on a new linter — or tightening rules — would flag thousands of pre-existing issues. If the build went red for all of them, the lint task would be unusable until every legacy violation was cleaned up. That is rarely practical.

At the same time, purely advisory linting ("soft" mode) is easy to ignore, and violations continue to pile up.

### Hold the line

The default strategy, `hold_the_line`, is a middle ground. It answers the question: *"Did this PR make things worse?"*

- On a PR build, the task compares lint errors against the set of files changed in the PR. If any error-severity diagnostics come from changed files, the task fails.
- On a non-PR build (e.g. a post-merge CI run on `main`), the task never fails, because there is no meaningful "changed file" set to compare against.

This lets teams adopt new linters immediately. Existing violations are tolerated, but new ones are blocked at the PR gate. Over time, the codebase improves organically as engineers touch files and fix violations along the way.

#### How hold the line is implemented

The mechanism relies on three things: PR detection, a git diff, and a two-tier filtering model.

**PR detection.** The task asks the host (e.g. the GitHub Actions host) whether this is a PR build. On GitHub Actions, this is determined by inspecting the `GITHUB_REF` environment variable. If it matches the pattern `refs/pull/<number>/merge`, the build is a PR build. On non-PR builds (branch pushes, scheduled runs, manual triggers), hold-the-line never fails — there is no changed-file set to compare against.

**Git diff.** The task lazily generates a diff by running `git diff HEAD~ HEAD` and writing the result to a temporary file. The diff is parsed into a list of `{ file, lines }` tuples, where `lines` contains the 0-based line numbers of *added* lines only (not removed or context lines). Deleted files are excluded entirely.

**Two-tier filtering.** The task applies two different predicates depending on the diagnostic type:

- *File-level matching* (used for annotations): A diagnostic matches if its source file path appears anywhere in the diff. This is a loose check — the error doesn't need to be on a changed line, just in a changed file.
- *Line-level matching* (used for suggestions): A diagnostic matches only if its source file *and* specific line number both appear in the diff. This is strict — the suggestion must point at a line the author actually added.

**The failure decision.** The `shouldFail` method receives two lists: `filteredDiagnostics` (only diagnostics from changed files/lines) and `allDiagnostics` (every diagnostic found). Under hold-the-line, it checks whether the build is a PR and whether `filteredDiagnostics` contains any error-severity entries. Under hard mode, it checks `allDiagnostics` instead. Under soft mode, it always returns false.

### Soft

The task never fails, regardless of how many lint errors exist. Diagnostics are still collected and surfaced as GitHub annotations/suggestions, but the build stays green.

### Hard

The task fails if *any* error-severity diagnostic exists, regardless of whether the error comes from a changed file. This is appropriate for codebases that are already clean and want to enforce zero lint errors at all times.

### Summary

| Strategy | Fails on PR builds? | Fails on non-PR builds? |
|---|---|---|
| `soft` | Never | Never |
| `hold_the_line` | Only if errors are from changed files | Never |
| `hard` | If any errors exist | If any errors exist |

## Bazel Invocation

The task constructs a Bazel command with two additional flags beyond what the base task provides:

- `--machine` — Requests structured (machine-readable) output from the CLI.
- `--lint_diagnostics_file=<path>` — Tells the CLI to write lint results as a JSON file to a temporary path. This file is the primary input for all downstream processing.

The temporary path is deterministic and namespaced by workspace and task ID to avoid collisions when multiple lint tasks run in the same pipeline.

## From SARIF to GitHub Comments

The path from linter output to GitHub annotations and suggestions spans three layers: the Aspect CLI (Go), the Rosetta lint task (TypeScript), and Marvin (the notification backend). This section traces the full journey.

### Layer 1: Aspect CLI — SARIF to diagnostics

Individual linters (eslint, flake8, etc.) produce SARIF reports and/or patch files. The Aspect CLI converts these into a unified diagnostics format before writing them to the JSON file specified by `--lint_diagnostics_file`.

**SARIF reports become annotations.** The CLI walks each SARIF run's results and locations, producing one diagnostic per finding. The severity is mapped from the SARIF level (`error` becomes `ERROR`, everything else becomes `WARNING`). The source file comes from the artifact location URI. The line number comes from the region's `startLine`. Each diagnostic gets `baggage["lint_result_type"] = "annotation"` and `baggage["label"]` set to the originating Bazel target.

**Patch files become suggestions.** When a linter emits a unified diff (a fix), the CLI parses the diff hunks and produces one diagnostic per hunk. The `help` field contains the suggested replacement text. The span's offset and height come from the diff line numbers. Each diagnostic gets `baggage["lint_result_type"] = "suggestion"`.

The result is a JSON file with a top-level `diagnostics` array containing both annotations and suggestions, uniformly represented as `DiagnosticInput` objects.

### Layer 2: Rosetta lint task — filtering and capping

This is where the bulk of the logic described in this document lives. The lint task reads the diagnostics file, normalizes workspace paths, filters diagnostics against the git diff, sorts them, caps them, and determines pass/fail. The details are covered in [Diagnostics Processing](#diagnostics-processing).

The output is a `lint-task-completed` event containing the processed diagnostics array, lint counts, repro/fix commands, and the pass/fail decision. This event is consumed by the Marvin listener.

### Layer 3: Marvin — diagnostics to GitHub

The Marvin listener receives the `lint-task-completed` event and forwards the diagnostics to the Marvin backend, which translates them into GitHub API calls.

**Annotations become GitHub check annotations.** Diagnostics with `lint_result_type = "annotation"` (or no type) are posted as annotations on the GitHub check run. The severity maps to an annotation level: `ERROR` becomes `failure`, `WARNING` becomes `warning`, and everything else becomes `notice`. The annotation includes the file path, line range, title, and message.

**Suggestions become GitHub PR review comments.** Diagnostics with `lint_result_type = "suggestion"` are posted as review comments on the pull request using the GitHub Pulls API. Each comment includes the file path, line number, span height (for multi-line suggestions), and the suggested replacement text from the `help` field. These render as GitHub's native suggestion blocks that the author can apply with one click.

### Controlling what gets posted

Two workflow-level configuration flags control whether diagnostics are surfaced on GitHub:

- `notifications.github.annotations` (default `true`) — When false, no check run annotations are created. The lint task short-circuits annotation processing entirely and returns an empty list.
- `notifications.github.suggestions` (default `true`) — When false, no PR review suggestion comments are created. The lint task short-circuits suggestion processing entirely.

A third flag, `notifications.github.show_aspect_cli_commands`, controls whether the repro and fix commands are included in the output. When false, these are omitted even if there are error-severity diagnostics.

## Diagnostics Processing

After Bazel finishes, the task reads the JSON diagnostics file. Processing happens in several stages.

### 1. Parse

The file contains a top-level `diagnostics` array. Each entry is a `DiagnosticInput` with fields like severity, message, title, source location, spans (line/column offsets), and a `baggage` map of arbitrary metadata.

Two pieces of baggage are significant for lint:

- `lint_result_type` — either `"annotation"` or `"suggestion"`. Annotations are "this is wrong" messages. Suggestions are "here is a fix" messages with replacement content.
- `label` — the Bazel label of the target that produced the diagnostic. Used to construct repro and fix commands.

### 2. Workspace path normalization

If the task runs in a subdirectory workspace (not the repo root), file paths in diagnostics need to be prefixed with the workspace path so they align with the repository-root-relative paths that GitHub expects.

### 3. Changed file detection

The task lazily generates a git diff (via `git diff HEAD~ HEAD`) that describes which files and lines changed in the current commit. The diff is parsed using the `parse-git-patch` library into a list of `{ file, lines }` tuples, where `lines` are the 0-based line numbers of added lines only.

- **Changed file**: a diagnostic's source file appears anywhere in the diff.
- **Changed line**: a diagnostic's source file *and* specific line number (from the span offset) appear in the diff. Additionally, the diagnostic must have exactly one span with a valid offset.

These two predicates are used differently depending on the diagnostic type (see below).

### 4. Split into annotations and suggestions

Diagnostics are partitioned by their `lint_result_type` baggage value.

**Annotations** (errors and warnings):
- Filtered to changed *files* (not individual lines) when `only_annotate_changed_regions` is true.
- When `only_annotate_changed_regions` is false, all annotations are included.
- Capped at **25** per task invocation to avoid flooding the PR.
- The `help` field is stripped from annotations before publishing (suggestions carry help, annotations don't need it).

**Suggestions** (auto-fixable issues):
- Always filtered to changed *lines* — suggestions are only relevant if they touch code the author just wrote.
- Capped at **10** per task invocation.
- The `message` field is stripped from suggestions before publishing (the suggestion content in `help` is the message).

### 5. Sorting

Both annotations and suggestions are sorted by a stable, multi-level comparator. The priority order (highest first) is:

1. Diagnostic is from a changed line
2. Diagnostic is from a changed file
3. Higher severity (error > warning > info)
4. File path (alphabetical)
5. Line number (ascending)
6. Title, message, help (alphabetical tiebreakers)

Because the lists are capped after sorting, this ordering ensures that the most relevant diagnostics — those closest to the author's changes and highest severity — survive the cut.

### 6. Repro and fix commands

If the workflow configuration has `show_aspect_cli_commands` enabled, the task constructs CLI commands users can run locally:

- **Repro command**: `bazel lint <target1> <target2> ...` — reproduces the errors locally. Only includes targets from error-severity diagnostics, deduplicated.
- **Fix command**: Same as the repro command with `--fix` appended — auto-applies fixes where possible.

## Error Handling

The task handles several error scenarios:

- **Non-lint Bazel failures** (exit codes other than 0 or the lint-specific failure code): The task fails immediately without attempting to parse diagnostics. No annotations or suggestions are produced.
- **Missing or unreadable diagnostics file**: The task fails. This typically indicates a CLI bug or a misconfigured pipeline.
- **Failed git diff parsing**: The task fails. A log diagnostic is emitted noting the failure.
- **Unexpected exceptions during diagnostic processing**: Caught and logged. The task fails conservatively.

In all error cases, the `lint-task-completed` event is still published with `successful: false` and empty diagnostic lists, so Marvin receives a consistent signal and can update the check run accordingly.

## Exit Code Mapping

| Scenario | Bazel exit code | Task outcome |
|---|---|---|
| No lint issues | `0` (OK) | Pass |
| Lint issues found, strategy says pass | Lint failure code | Pass (exit code overridden to OK) |
| Lint issues found, strategy says fail | Lint failure code | Fail |
| Bazel internal error | Any other code | Fail |
