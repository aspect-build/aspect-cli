# Bazel invocation API — context & decisions

Building one higher-level, composable API for driving Bazel from tasks, replacing
the per-task copy-pasted spawn/drain glue. Goal: a new task adopts it by importing,
and the flow reads top-to-bottom at the callsite.

## Decisions (settled)

- **One composable API**: `bazel.build(ctx)` / `bazel.test(ctx)` invocation handles.
  build.axl / test.axl stay tidy — the whole flow (setup → spawn → events → wait →
  retry → conclude) is visible at the callsite.
- **Separate constructors per command** — never a command-string/enum parameter.
- **Plain AXL library, not a second feature system.** It consumes the existing
  `BazelTrait`; features keep extending builds through the trait.
- **Pull-based event consumption, no callbacks, no pump at the callsite.** The
  callsite loops over the event stream directly; cadence lives in the runtime
  (the iterator yields on a tick so heartbeats fire even when Bazel is quiet).
- **Use the type system** (records/enums/annotations). But don't invent redundant
  types: reuse the runtime's existing status value, and treat targets as a plain
  list.
- **Explicit retry decision at the callsite** (`build.should_retry(...)`), not
  hidden inside wait.
- **Args exposed as shared constants** that tasks splat in; targets included.
- **New runtime (Rust) APIs are acceptable** and are wanted — keep them.
- **Keep the Rust-side changes.** (Confirmed during the revert discussion.)
- **Keep the whole change** as it currently stands — it's the landable baseline.

## Still to do (decided, not yet done)

- **Featurize deployment and repro.** Move `--deployment` / `--aspect-remote*` and
  `--repro-flavors` into the feature system. Accepted consequences: the flags get
  renamed (feature-namespaced) and are injected into every command — this breaking
  change is OK.
- **Migrate the remaining Bazel-spawning tasks** (run, format, lint, warming,
  gazelle, delivery) onto the same invocation API so there's truly one path.
  Delivery is the hard one — it runs multiple invocations with different Bazel
  servers, so the API needs to allow a per-invocation run-configuration override
  before delivery can move over.

## Notes

- There is a pre-existing bug on this branch (unrelated to this work): some tasks
  reference the builtin Bazel event API through a name that the branch's namespace
  refactor no longer exposes. Migrating those tasks (above) removes the broken call
  sites; until then they need the aliasing fix the other tasks already use.
