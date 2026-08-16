## Bazel foundations

Repository-declared versions win over this catalogue's recommendations. Use a catalogue version only where the repository states nothing, and report which values were derived versus chosen as defaults. Preserve an existing Bazel version during an initial migration; make a major-version upgrade a separate change.

Where the repository declares no Bazel version — the usual case when no Bazel root exists — pin `9.2.0`. Write `.bazelversion` before running any Bazel command, so that every command in the migration, including the first `bazel --version`, runs under the version the repository will keep; do not bootstrap through `USE_BAZEL_VERSION` or whatever plain `bazel` is on `PATH`, which makes the result unreproducible for CI and for the next agent to touch the repository.

A module's "tested with" Bazel version, whether from its registry page or its own CI, is a lower bound on what its maintainers exercised, not a ceiling. It does not select the repository's Bazel version and is not grounds for lowering it. Depart from `9.2.0` only for an incompatibility you actually hit — a build failure you observed, or a `bazel_compatibility` bound declared in the module — and record which one, with the evidence, in the report.

Create a root `BUILD.bazel` before a module extension refers to a root label such as `//:go.mod`, `//:Cargo.toml`, `//:pyproject.toml`, or `//:maven_install.json`.

## Git ignore policy

Ignore Bazel's root-level convenience symlinks and private developer overrides, but keep the build's declared state in version control:

```gitignore
/bazel-*
/.bazelrc.user
```

Do not ignore or delete `MODULE.bazel`, `MODULE.bazel.lock`, `BUILD.bazel`, `.bazelrc`, `.bazelversion`, or `.bazelignore`.

## Validation

A stage is complete only after a representative build and test. `Executed 1 out of 1 test` counts targets, not test cases, and prints identically for a target that discovered nothing. The real count is in `bazel-testlogs/<package>/<target>/test.log`, or on the console with `--test_output=all`: `running 105 tests` from Rust's libtest, `Tests: succeeded 74` from ScalaTest, `12 passed` from pytest. Confirm a non-zero count there; a target that discovers no tests is not validated. Report any native-build comparison that could not be run and why.
