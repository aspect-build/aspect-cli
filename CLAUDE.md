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
