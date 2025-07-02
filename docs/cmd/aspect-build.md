# `aspect build`

The `build` command exists to align with `bazel build` and other build tool invocations.

`build` should do one thing -- lay down build results.

## Design goals
- We want `build` to present a minimal interface that does one thing
- `build` should be able to process generalized label queries, not just `...` and `:*` and soforth
- `build` should NOT go flag for flag with `bazel build` because doing so would be laborious and difficult to do so compatibly
- `build` should provide `bazelisk`-equivalent behavior by either using a local `bazelisk` or fetching and punting to a released version
