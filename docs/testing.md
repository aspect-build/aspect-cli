# AXL native testing — design sketch + POC

Status: **proof-of-concept**. This document captures the design we converged
on for giving AXL a first-class, pytest-style testing story built into the
engine, describes what the POC in this branch actually implements, and logs
the decisions made without explicit sign-off so they can be reviewed.

## Goal

A test author writes:

```python
# lib/ci_test.axl   — a *_test.axl file gets the augmented test surface
load("./ci.axl", "detect_ci_host")

def test_detect_ci_host_none_off_ci(t):
    asserts.eq(detect_ci_host(t.ctx.std.env), None)

def test_github_actions_precedence(t):
    t.env.set("GITHUB_ACTIONS", "true")
    t.env.set("BUILDKITE", "true")
    asserts.eq(detect_ci_host(t.ctx.std.env)["marker"], "GITHUB_ACTIONS")
```

and runs `aspect test //...`. No per-test wiring in `config.axl`, no
hand-maintained list in `pipeline.yaml`, no copied `_eq` helper, no
`_snapshot_env`/`_restore_env` dance.

## The converged design

1. **`*_test.axl` files get a different globals surface.** The loader
   evaluates files whose name ends in `_test.axl` against *base AXL + a
   test-only vocabulary*. The extra names exist **only** in test files, so
   test scaffolding can never leak into production `config.axl` / builtins.

2. **Tests are functions, discovered by convention.** A test is a top-level
   `def test_*(t)`. The runner enumerates a module's `test_*` callables — the
   same shape as the existing task discovery (`FrozenTaskModuleLike::tasks()`
   filters module names by value kind).

3. **The harness `t` is bazel-free and purpose-built.** Each test gets a
   handle `t` with:
   - `t.env` — an in-memory environment overlay (set/get/remove/reset),
   - `t.std` — the real `std` surface (fs, env, io, …),
   - `t.ctx` — a **real `TaskContext`** (the same Rust type production uses),
     wired over the mock backends.

   `t` has **no bazel surface**. `TaskContext` is the bazel-flavored context;
   we mint a narrower context for tests, exactly as the engine already mints
   different contexts per phase (`ConfigContext`, `FeatureContext`).

4. **Mocking is backend-swap, not type masquerade.** `t.ctx.std.env` is the
   genuine `std.Env` type. It reads the in-memory overlay only because the
   runner installs a `test_env` on `eval.extra` for the duration of the test.
   The Starlark type and its method table are unchanged; only the backend they
   consult differs. This keeps the mock's contract *identical* to reality
   (enforced by the type system) instead of hand-maintaining a parallel
   look-alike type — and it keeps every internal `downcast_ref::<RealType>()`
   working, which a masquerading type would silently break.

5. **Per-test isolation, pytest semantics, run in parallel.** Each test runs
   with a fresh overlay. A failed assertion raises; the runner catches it
   per-test, records the failure, and continues to the next test. Tests fan
   out across `min(tests, cpus)` worker threads (overridable, `--jobs`-style),
   each with its own Starlark heap — heaps are `!Send`, so workers re-evaluate
   the side-effect-free module body locally rather than sharing one. Results
   merge back into definition order so the report is deterministic. This is
   sound **because** every per-test fixture lives on the test's own values
   (the env overlay, and the bazel backend below), never in a process-global —
   concurrent workers therefore share no mutable state.

## What the POC implements (and how to run it)

All in `crates/axl-runtime`:

| Piece | Location |
|---|---|
| `asserts` namespace (`eq`, `ne`, `is_true`, `is_false`, `contains`, `fails`) | `src/engine/testing.rs` |
| `Test` harness value (`t.env`, `t.std`, `t.ctx`) | `src/engine/testing.rs` |
| `test_*` discovery + parallel (thread-per-shard) runner + summary | `src/engine/testing.rs` (`run_test_source`) |
| In-memory env overlay handle (`TestEnvMap`) carried on the harness values | `src/engine/store.rs` (`TestEnvMap`); minted in `src/engine/testing.rs` |
| `std.Env`/`Std`/`TaskContext` carry the overlay (`Option<TestEnvMap>`) on the value | `src/engine/std/env.rs`, `src/engine/std/mod.rs`, `src/engine/task_context.rs` |
| `std.env` reads/writes the overlay carried on its value when present | `src/engine/std/env.rs` (`var`/`set_var`/`remove_var`/`vars`) |
| Test-only globals surface | `src/eval/api.rs` (`get_test_globals`) |
| Loader selects test globals for `*_test.axl` | `src/eval/load.rs` |

Run the end-to-end proof:

```sh
cargo test -p axl-runtime testing::
```

`discovers_and_runs_test_functions` proves: test-only `asserts` parses only via
the test surface; `test_*` discovery (and that `helper_*` is *not* run);
`t.env` overlay observed through both `t.std.env` and a real `t.ctx`;
isolation; and per-test failure capture. `overlay_does_not_leak_into_process`
proves the overlay never mutates the real process environment.
`runs_tests_in_parallel_shards` forces the multi-worker path (8 jobs over 17
tests) and proves cross-test isolation holds concurrently and that outcomes
still merge back into definition order.

## Decisions — reviewed

Items marked *Decided* were reviewed and settled during the design walkthrough;
the rest were forced or chosen to keep the POC moving and remain cheap to
change:

1. **`asserts`, not `assert`.** `assert` is a **reserved keyword** in the
   AXL/Starlark dialect and cannot be used as an identifier, so the namespace
   `assert.eq(...)` won't parse. Per review, the plural `asserts` is used
   (`asserts.eq`, `asserts.contains`, …) — it parses and reads almost exactly
   like `assert.*`. Alternatives considered: a different namespace (`check`,
   `expect`), or moving assertions onto the harness (`t.eq(...)`,
   `t.assert_eq(...)`).

2. **Assertions are a global namespace, not harness methods.** *Decided:* keep
   `asserts.*` global. The alternative (`t.assert_eq`) is viable but the
   test-globals swap is the mechanism that scopes *future* test-only builders
   (`parametrize`, fixtures), so we keep it and free-standing `asserts.eq`
   reads cleaner than threading `t.` through every check.

3. **`contains` is string-only in the POC.** Collection membership
   (`needle in haystack`) is stubbed out with a clear error. Trivial to extend.

4. **`is_true`/`is_false`, not `true`/`false`.** Rust reserved words; also
   clearer.

5. **The runner re-evaluates the test source in a live module** rather than
   reusing the loader's frozen module + cross-heap calls. Simpler and avoids
   freeze/thaw; the loader globals-swap is still implemented and exercised for
   the normal load path. A production runner should unify these.

6. **`env` is an *overlay*, not a "backend"; bazel *is* a backend.** *Decided
   vocabulary.* For env, the `std.Env` type and methods are identical — only
   the map they read swaps — so "overlay" fits. For bazel, `Real` (spawns a
   subprocess, live BES) and `Fake` (below) are genuinely different
   implementations of one contract, so `BazelBackend::{Real, Fake}` fits.
   *Decided mechanism:* both the overlay handle and the bazel backend are
   **carried on the harness-constructed value**, not bolted onto the production
   `Env` and fished out of `eval.extra`. *Done for env (roadmap item 1c):* the
   overlay is a `TestEnvMap` (`Arc<Mutex<BTreeMap<…>>>`) carried as an
   `Option` on the `std.Env` / `Std` / `TaskContext` values. The runner mints
   the harness's `t.env`, `t.std`, and `t.ctx` from one shared handle, so all
   three observe the same map; production mints these values with `None` and
   `std.env` hits the real process env unchanged. `Env::test_env` /
   `with_test_env` and the `from_eval` mock route are gone. (`Arc<Mutex>`, not
   `Rc<RefCell>`, because the values that carry it must satisfy the `Send +
   Sync` bound frozen Starlark values require; each overlay is still only
   touched on its own worker thread, so the mutex is never contended.)

7. **bazel `Fake` = a generic fake-bazel process driven by declared data.**
   *Decided, superseding the earlier "canned `BuildEvent`s" sketch.* Canned
   in-process events were rejected for two reasons: (a) they'd force us to
   hand-reimplement *every* surface AXL consumes — BES (`BuildEventIter`),
   execlog (`ExecLogSink`, incl. the zstd `CompactFile` format), stdout/stderr
   streams, exit codes — and keep them mutually consistent; (b) baking named
   scenarios into Rust (as `basil` does) means a test author can't express new
   behavior without editing Rust, which breaks the no-Rust promise. Instead:
   - a **single generic fake-bazel** *synthesizes* all surfaces from a declared,
     typed AXL fixture (a `BazelExpectation` record: `targets` / `result` enum /
     `exit_code`, with a raw `events=` escape hatch). Author writes intent; the
     fake manufactures consistent BES + execlog + streams + exit so they can't
     drift.
   - **Control channel = an inherited `socketpair`** (parent → fake) carrying
     the serialized fixture; bidirectional so timing/cancellation tests can
     drive the fake mid-stream (the lifecycle fidelity canned events can't
     reach). Unix-only — the control transport sits behind a small trait so a
     Windows named-pipe/loopback impl is a drop-in later.
   - **Output channels are the *real* bazel channels** the parent already wires
     for real bazel (`--build_event_binary_file`/gRPC, `--execution_log_*`,
     stdout/stderr), so the production `ctx.bazel.build` read path is exercised
     unchanged. Every per-invocation resource (socketpair, BES path/port,
     execlog path, temp dir) is **uniquely derived per spawn** — a hard
     requirement under the parallel runner.
   - **Process model = fork+exec (`current_exe`/`posix_spawn`), never bare
     `fork()`.** We carry a tokio runtime + threads; a bare `fork()` is unsafe
     (frozen locks, broken runtime). `exec` is what gives the "start fresh"
     image; inherited FDs (the socketpair, BES file) give fork's only useful
     property without its hazard.
   - **Reuse `basil`, don't reinvent.** `basil` already *is* the separate
     fake-bazel binary (spawned via `BAZEL_REAL`, writes real length-delimited
     `BuildEvent`s). Plan: extract its replay/synthesis into a `basil-core` lib;
     standalone `basil` stays for the Rust integration tests; shipped
     `aspect test` reuses the lib via a hidden self-exec subcommand. We do
     **not** `include_bytes!` a standalone `basil` (≈2–3 MB stripped, mostly
     `prost`/proto already linked into aspect-cli) — that duplicate is the
     binary bloat to avoid.

8. **Parallelism makes "state on the value, nothing global" a correctness
   requirement, and turns three in-tree shortcuts into bugs to fix before the
   bazel `Fake` lands:**
   - `std::env::set_var("BAZEL_REAL", …)` (`test.rs`) is process-global → the
     `Fake` backend must build the `Command` with the fake path directly
     (carried on the value), not flip a global env var.
   - the BES output path / gRPC port and execlog path must be per-invocation
     unique (fixed paths/ports collide across concurrent builds).
   - the spawn registry (`bazel/live.rs`, `static REG`) pools pids from *all*
     concurrent tests → cancellation scope must be per-test, not the global
     registry.
   `t.stdout()` (roadmap) must likewise be a per-test buffer, never a
   process-stdout redirect.

9. **No `aspect test` CLI task yet.** The runner is exposed as a Rust function
   and proven by Rust tests. The design calls for `aspect test` to be a
   builtin **AXL task** (next to `axl_add.axl`) that calls a sandbox-run
   primitive — wiring that touches `cmd.rs` + `MODULE.aspect` and is the next
   step, deliberately out of this slice. The fake-bazel embedding (item 7's
   self-exec subcommand) rides on this step.

10. **No deny-by-default for unstubbed net/process.** That hermeticity property
    (an unstubbed subprocess/HTTP call fails the test) is designed but not
    built, since those backends aren't mocked yet.

## Roadmap / open questions

Build order (each independently shippable):

1. ✅ `env` overlay + `asserts` + discovery + `t.ctx` (this POC).
1b. ✅ **Parallel runner** — thread-per-shard, `min(tests, cpus)` workers,
    deterministic merge (`run_test_source` / `run_test_source_with_jobs`).
1c. ✅ Move the env overlay off the production `Env`/`eval.extra` and **onto the
    `std.Env` value** (`Option<overlay>`); `t.env` and `t.ctx.std.env` share one
    handle. Removed `Env::test_env` + the `from_eval` mock route. (Handle is
    `Arc<Mutex<…>>` so the value satisfies the `Send + Sync` bound frozen
    Starlark values require.)
2. ✅ `bazel` → `BazelBackend::{Real, Fake}` on the `bazel.Bazel` value (carried
   on the value, read via `read_backend`, not `eval.extra`). `Fake` fork+execs a
   generic fake-bazel (`basil-core`, reused via the standalone `basil` binary
   today) with a per-invocation `socketpair` control channel carrying a
   length-delimited `BazelExpectation` fixture; the fake synthesizes a
   consistent `BuildStarted` → `TargetComplete`* → `BuildFinished` BES stream +
   exit code onto the real `--build_event_binary_file` the parent already wires,
   so the production `ctx.bazel.build` read path is exercised unchanged. `Fake`
   builds the `Command` straight from the fake path — no `BAZEL_REAL` global —
   and derives the child pid as galvanize's `server_pid`. `t.bazel.expect_build(
   *targets, result=, exit_code=)` declares the fixture. (See decisions 7/8.)
   *Not yet synthesized from the typed fixture:* execlog + stdout/stderr (BES +
   exit only); a raw `events=` escape hatch passes pre-framed `BuildEvent`s
   through. Unix-only — the control transport sits behind a `ControlChannel`
   trait so a Windows named-pipe impl is a drop-in.
3. `io` backend → captured `t.stdout()` (per-test buffer, never process stdout).
4. `fs` backend → `t.fs.tmpdir()` (tmpdir-rooted real fs by default).
5. `process` / `net` backends → `t.process.stub(...)` / `t.http.stub(...)`,
   **deny-by-default**.
6. `aspect test` as an AXL runner task + the sandbox-run engine primitive;
   extract `basil-core` and ship the fake-bazel via a hidden self-exec
   subcommand (no embedded second binary — see decision 7).
7. Snapshots: `t.snapshot(value, name=...)` + golden files + `--update`.
8. Teach `axl-lsp` / `axl-docgen` about the `_test.axl` augmented surface.

Open questions to settle before promoting past POC:
- ✅ Namespace name — `asserts`, global (decisions 1, 2).
- ✅ env "overlay" vs bazel "backend"; state carried on the value (decision 6).
- ✅ bazel `Fake` shape — generic process + socketpair + synthesized surfaces,
  reusing basil-core; not canned events, not Rust scenarios (decisions 7, 8).
- ✅ `BazelExpectation` control-channel **wire format** — length-delimited
  protobuf (the framing basil already uses for BES); the `events=` escape hatch
  carries pre-framed `BuildEvent`s as opaque `bytes` passed through untouched.
- Snapshot golden location: `__snapshots__/` dir vs inline-string snapshots.
- Should the test surface also gate on *where* the runner loads from, so a
  stray `_test.axl` evaluated outside the runner can't pick up test globals?
- Fixtures: single-`t` + helpers + `t.defer` for v1, or signature-injected
  named fixtures + `parametrize` later?
