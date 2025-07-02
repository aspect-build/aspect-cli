# `aspect run`

As in Bazel et all `run` should be `build` + `fork()`

## Design goals
- It'd be nice if we could relax the "runnable" constraints so that you can `run --run_under=cat <file>`
- As in Bazel we want `run` to defer to `build` as much as possible and just do something with the results
