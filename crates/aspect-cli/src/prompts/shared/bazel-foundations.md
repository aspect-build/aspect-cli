## Bazel foundations

Repository-declared versions win over this catalogue's recommendations. Use a catalogue version only where the repository states nothing, and report which values were derived versus chosen as defaults. Preserve an existing Bazel version during an initial migration; make a major-version upgrade a separate change. Where the repository declares none — the usual case when no Bazel root exists — create `.bazelversion` holding the version you actually validated with, from `bazel --version`, and commit it. Leaving selection to whatever is on `PATH` makes the result unreproducible for CI and for the next agent to touch the repository.

Create a root `BUILD.bazel` before a module extension refers to a root label such as `//:go.mod`, `//:Cargo.toml`, `//:pyproject.toml`, or `//:maven_install.json`.

## Git ignore policy

Ignore Bazel's root-level convenience symlinks and private developer overrides, but commit the build's declared state:

```gitignore
/bazel-*
/.bazelrc.user
```

Do not ignore or delete `MODULE.bazel`, `MODULE.bazel.lock`, `BUILD.bazel`, `.bazelrc`, `.bazelversion`, or `.bazelignore`.

## Validation

A stage is complete only after a representative build and test. `Executed 1 out of 1 test` counts targets, not test cases, and prints identically for a target that discovered nothing. The real count is in `bazel-testlogs/<package>/<target>/test.log`, or on the console with `--test_output=all`: `running 105 tests` from Rust's libtest, `Tests: succeeded 74` from ScalaTest, `12 passed` from pytest. Confirm a non-zero count there; a target that discovers no tests is not validated. Report any native-build comparison that could not be run and why.
