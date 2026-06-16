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

5. **Per-test isolation, pytest semantics.** Each test runs with a fresh
   overlay. A failed assertion raises; the runner catches it per-test, records
   the failure, and continues to the next test.

## What the POC implements (and how to run it)

All in `crates/axl-runtime`:

| Piece | Location |
|---|---|
| `asserts` namespace (`eq`, `ne`, `is_true`, `is_false`, `contains`, `fails`) | `src/engine/testing.rs` |
| `Test` harness value (`t.env`, `t.std`, `t.ctx`) | `src/engine/testing.rs` |
| `test_*` discovery + isolated runner + summary | `src/engine/testing.rs` (`run_test_source`) |
| In-memory env overlay backend | `src/engine/store.rs` (`Env::test_env`, `with_test_env`) |
| `std.env` reads/writes the overlay when present | `src/engine/std/env.rs` (`var`/`set_var`/`remove_var`/`vars`) |
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

## Decisions made without explicit sign-off — please review

These were forced or chosen to keep the POC moving; none are load-bearing
commitments and all are cheap to change:

1. **`asserts`, not `assert`.** `assert` is a **reserved keyword** in the
   AXL/Starlark dialect and cannot be used as an identifier, so the namespace
   `assert.eq(...)` won't parse. Per review, the plural `asserts` is used
   (`asserts.eq`, `asserts.contains`, …) — it parses and reads almost exactly
   like `assert.*`. Alternatives considered: a different namespace (`check`,
   `expect`), or moving assertions onto the harness (`t.eq(...)`,
   `t.assert_eq(...)`).

2. **Assertions are a global namespace, not harness methods.** Consistent with
   (1); the alternative (`t.assert_eq`) is equally viable and would remove the
   need for the test-globals swap entirely. Kept the swap because it's the
   mechanism that scopes *future* test-only builders (`bes.*`, `parametrize`).

3. **`contains` is string-only in the POC.** Collection membership
   (`needle in haystack`) is stubbed out with a clear error. Trivial to extend.

4. **`is_true`/`is_false`, not `true`/`false`.** Rust reserved words; also
   clearer.

5. **The runner re-evaluates the test source in a live module** rather than
   reusing the loader's frozen module + cross-heap calls. Simpler and avoids
   freeze/thaw; the loader globals-swap is still implemented and exercised for
   the normal load path. A production runner should unify these.

6. **Only the `env` backend is mocked so far.** It's the smallest surface and
   the one every existing suite needs (`ci_test.axl` etc.). `io`/`fs`/`net`/
   `process`/`bazel` follow the identical pattern (see roadmap) but are not in
   the POC.

7. **No `aspect test` CLI task yet.** The runner is exposed as a Rust function
   and proven by a Rust test. The design calls for `aspect test` to be a
   builtin **AXL task** (next to `axl_add.axl`) that calls a sandbox-run
   primitive — wiring that touches `cmd.rs` + `MODULE.aspect` and is the next
   step, deliberately out of this slice.

8. **No deny-by-default for unstubbed net/process.** That hermeticity property
   (an unstubbed subprocess/HTTP call fails the test) is designed but not
   built, since those backends aren't mocked yet.

## Roadmap / open questions

Build order (each independently shippable, all reuse the
`from_eval(eval).<sub>()` backend-routing pattern):

1. ✅ `env` backend + `asserts` + discovery + runner + `t.ctx` (this POC).
2. `io` backend → captured `t.stdout()` for output assertions.
3. `fs` backend → `t.fs.tmpdir()` (tmpdir-rooted real fs by default).
4. `process` / `net` backends → `t.process.stub(...)` / `t.http.stub(...)`,
   **deny-by-default**.
5. `bazel` → `BazelBackend::{Real, Fake}` enum; `Fake` feeds canned
   `BuildEvent` protos into the existing `BuildEventIter` channel (basil stays
   the process-level / contract-pinning tier).
6. `aspect test` as an AXL runner task + the sandbox-run engine primitive.
7. Snapshots: `t.snapshot(value, name=...)` + golden files + `--update`.
8. Teach `axl-lsp` / `axl-docgen` about the `_test.axl` augmented surface.

Open questions to settle before promoting past POC:
- Namespace name (`asserts` vs `check`/`expect` vs harness methods) — decision (1).
- Snapshot golden location: `__snapshots__/` dir vs inline-string snapshots.
- Should the test surface also gate on *where* the runner loads from, so a
  stray `_test.axl` evaluated outside the runner can't pick up test globals?
- Fixtures: single-`t` + helpers + `t.defer` for v1, or signature-injected
  named fixtures + `parametrize` later?
