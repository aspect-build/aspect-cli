# `aspect test`

As in Bazel et all `test` should be `build` and also build exit codes.

## Design goals
- Bazel separates out coverage, we want a better plan for that
- As with `build`, it'd be nice if we could directly supply queries to `test` rather than doing `query | xargs test` dances
