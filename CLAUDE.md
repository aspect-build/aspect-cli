Guideline for writing AXL code lives at ./docs/axl.md

## Writing to stdout/stderr

**Never use `println!`, `eprintln!`, `print!` or `eprint!` in CLI code.** They
panic when the write fails, and a failed write is routine: `aspect build … |
head`, or a CI assertion piping into `grep -q`, closes the pipe as soon as the
reader has what it wants. The panic aborts the process *mid-task*, so nothing
runs the task's terminal update and its GitHub check run and PR-comment entry
are stranded showing "running" — a finished task reported as in progress for as
long as the comment lives.

Use the tolerant macros from `axl_runtime::out` instead. They discard a failed
write: output past the close is lost either way, since nobody is reading it,
but the process finishes its work and exits with its real code.

| Instead of | Use |
|---|---|
| `println!` | `outln!` |
| `eprintln!` | `errln!` |
| `print!` | `out!` |

```rust
use axl_runtime::{errln, outln};

outln!("{}", rendered);
errln!("warning: {msg}");
```

Within `axl-runtime` itself, import them as `crate::outln` / `crate::errln`.

### When a failed write *is* an error

The macros are for **human-facing console output only**. When the bytes are a
protocol payload that another program parses — a credential-helper response, a
machine-readable dump — a failed write means the consumer got nothing, and
reporting success is worse than the panic it replaced. Propagate it:

```rust
writeln!(stdout, "{response}").context("writing the credential-helper response")?;
stdout.flush().context("flushing the credential-helper response")?;
```

Flush as well as write: stdout is line-buffered, so a write with no trailing
newline is buffered and reports success, and the `BrokenPipe` first surfaces at
the flush.

### Other paths that bypass the macros

Two writers do not go through Rust's print macros at all, and both needed
fixing separately. Keep them in mind when adding a new output path:

- **AXL `print()`** goes through starlark's `PrintHandler`, whose default is a
  bare `eprintln!`. Every `Evaluator` must get
  `eval.set_print_handler(&axl_runtime::out::TOLERANT_PRINT_HANDLER)`.
- **`ctx.std.io.stdout` / `.stderr`** from AXL are `axl_types::stream::Writable`
  handles; their `write` and `flush` route a `BrokenPipe` through
  `ignore_broken_pipe`. A `File` or a child's stdin still propagates, because
  those errors are real.

A subprocess that inherits the console fd (see `writable_to_stdio`) receives
`SIGPIPE` itself when the reader leaves — that is the child's exit status, not
something these macros can absorb.

### Testing

`no_panicking_print_macros` (a unit test in each crate) scans that crate's own
sources and fails on a reintroduced macro, naming the file and line. It runs
under Bazel, so it is enforced in CI.

`crates/aspect-cli/tests/broken_pipe.rs` covers the behavior. Its tests close
the child's pipe *before* it writes, so the first write fails deterministically
rather than depending on the buffering race a real pipeline hits.

Two things to know when adding tests here:

- **Exercise the path you changed.** A test against `--help` passes whether or
  not the fix works, because clap writes and error-checks its own help. Confirm
  the test fails with the fix reverted before trusting it.
- **`rust_test(crate = …)` runs unit tests only.** Anything under `tests/`
  needs its own `rust_test` target with `srcs`, or it never runs — CI has no
  `cargo test` step. Resolve the binary through `ASPECT_CLI_BIN` (set by the
  rule's `env`) with an `option_env!("CARGO_BIN_EXE_…")` fallback for cargo;
  plain `env!` will not compile under Bazel.

## Ending a task early: refusals versus bugs

An error that escapes a task renders with the AXL traceback and an annotated
source snippet. That is the right output for a **bug**. For an **expected
refusal** whose message is the whole story (an unknown deployment, a missing
login, nothing to do), end the task with the message alone. The runtime prints
it as an `ERROR:` line (`WARNING:` when flagged, `INFO:` for exit code 0),
then the usual closing bookend, and exits with the code. `ctx.defer` callbacks
still run. `ASPECT_DEBUG=1` appends the traceback for anyone debugging.

**From AXL**, pick by where you are:

| Where | Use |
|---|---|
| Top of `_impl` | `return TaskConclusion(exit_code = 1, message = "...")` |
| Any nested helper | `ctx.std.process.exit(1, "...")` |
| A refusal a library names | `fail(NotLoggedIn("..."))`, with `NotLoggedIn = error.type(traceback = False)` |
| A bug, anywhere | `fail("...")`, which keeps the traceback |

`exit` takes any code 0..=255 and unwinds through every caller like an error,
so whatever the body had yet to run is skipped. Anything that must happen
regardless goes in a post-task hook (below); every `phases.new` handle closes
its status surface that way. `docs/axl.md` §13 has the examples.

**From Rust**, a `#[starlark_module]` fn returns
`axl_runtime::TaskExit::error(msg)` as its `anyhow::Error` instead of a plain
`anyhow!`; `TaskExit::new(code, message)` picks another code. Keep it at the
root of the error, not under `.context(...)`, or the downcast misses it and
the error renders as a traceback. The `unknown deployment` refusals in
`engine/aspect/auth.rs` are the pattern to copy.

An AXL error value whose type has `traceback = False` is the third producer:
`fail(e)` raises it as an `engine/error.rs` `RaisedError` carrying a
`TaskExit` (code 1), and `TaskExit::from_starlark` / `from_anyhow` find it
there, so everything below applies to it unchanged. `docs/axl.md` §15
describes error values.

**Where it is caught.** A `TaskExit` raised inside the task body is resolved
by the task runner in `eval/multi_phase.rs` into the same `Outcome` a returned
`TaskConclusion` produces (`eval/outcome.rs`). One raised from a feature or
config impl never reaches the runner; the top-level error arm in
`aspect-cli/src/main.rs` recognizes it instead. Both paths have tests:
`eval/exit.rs` for the runtime, and `crates/aspect-cli/tests/task_exit.rs`
for the real binary, one command per path. A runtime test that needs a feature
impl to run opts in with `.with_features(&["Name"])` on the test harness.

## Task hooks

`ctx.hooks` (`engine/task_hooks.rs`) is one `TaskHooks` value shared by
`config.axl`, every feature impl, and the task body. `pre_task(fn)` runs
`fn(ctx)` before the body; `post_task(fn)` runs `fn(ctx, conclusion)` after
it, however it ended, including a hard error, whose conclusion carries exit
code 1 and the error's summary before the error propagates. The runner's
order is pre-task hooks, body, post-task hooks, `ctx.defer` callbacks,
bookend. Post-task hooks see the exit code after the unclaimed-passthrough-flag
check. A failing post-task hook or `ctx.defer` callback is a `WARNING:` and
changes nothing; both go through `task_hooks::report_callback_failure`.

Use a post-task hook, not `ctx.defer`, for anything that needs to know how the
task ended. `lifecycle.axl`'s `phases.new` registers one per handle: if the
task ends before its own final `phases.update`, the hook sends a terminal
update with the runtime's verdict, so no GitHub check, Buildkite annotation,
or GitLab status is left on "running". `docs/axl.md` §14 has the AXL-facing
description.
