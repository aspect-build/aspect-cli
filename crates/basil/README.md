# basil

A fake `bazel` binary used to drive integration tests of `ctx.bazel.build`.

## What it does

`basil` mimics just enough of Bazel's CLI to satisfy `axl-runtime`. Tests point the runtime at it via `BAZEL_REAL=<path-to-basil>` (the bazelisk convention) and basil pretends to be Bazel for the duration of the call.

It supports two verbs:

- `basil info <key>...` — prints the small set of `key: value` lines the runtime parses (`server_pid`, `release`, `output_base`).
- `basil build` / `basil test` — writes a canned sequence of Build Event Protocol messages to the path passed in `--build_event_binary_file`, then exits.

## Selecting behavior

The interesting part is the `--scenario=<name>` flag. Each scenario is a named, hand-crafted sequence of build events that reproduces a specific Bazel behavior or known runtime bug. AXL tests pick one inline:

```python
build = ctx.bazel.build(flags = ["--scenario=success"], build_events = True)
```

Scenarios live in `src/main.rs`. Add new ones there and document what they reproduce. Names should read clearly at the call site (`success`, `bug1`, `cache_evicted_with_retry`).

## When to use it

Use basil any time a test wants to exercise the `ctx.bazel.build` path without spawning real Bazel. It's deterministic, fast, and lets you reproduce edge cases (cache eviction, broken pipes, retry storms) that are hard to trigger against a live Bazel server.

It is not a replacement for end-to-end testing against real Bazel — only for unit-level coverage of axl-runtime's BES handling.
