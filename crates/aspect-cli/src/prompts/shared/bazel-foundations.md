## Bazel foundations

Repository-declared versions win over this catalogue's recommendations. Use a catalogue version only where the repository states nothing, and report which values were derived versus chosen as defaults. Preserve an existing Bazel version during an initial migration; make a major-version upgrade a separate change.

Create a root `BUILD.bazel` before a module extension refers to a root label such as `//:go.mod`, `//:Cargo.toml`, `//:pyproject.toml`, or `//:maven_install.json`.

## Git ignore policy

Ignore Bazel's root-level convenience symlinks and private developer overrides, but commit the build's declared state:

```gitignore
/bazel-*
/.bazelrc.user
```

Do not ignore or delete `MODULE.bazel`, `MODULE.bazel.lock`, `BUILD.bazel`, `.bazelrc`, `.bazelversion`, or `.bazelignore`.

## Validation

A stage is complete only after a representative build and test. Confirm that the test log reports a non-zero test count; a target that discovers no tests is not validated. Report any native-build comparison that could not be run and why.
