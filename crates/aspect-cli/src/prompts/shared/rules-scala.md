## Scala recipe

Use `rules_scala` `7.2.6` as the Bzlmod starting point. Preserve the repository's declared Scala line, cross-version matrix, compiler options, semanticdb or compiler-plugin setup, generated sources, and sbt dependency policy; do not silently collapse a Scala 2/Scala 3 or cross-built project into one version. `rules_scala`'s default is Scala 2.12, so selecting the repository's actual version is mandatory.

Start `MODULE.bazel` from this shape after inspecting the existing build and creating a root `BUILD.bazel`:

```starlark
bazel_dep(name = "rules_scala", version = "7.2.6")

scala_config = use_extension(
    "@rules_scala//scala/extensions:config.bzl",
    "scala_config",
)
scala_config.settings(scala_version = "<declared-scala-version>")

scala_deps = use_extension(
    "@rules_scala//scala/extensions:deps.bzl",
    "scala_deps",
)
scala_deps.scala()
scala_deps.scalatest()
```

Every toolchain is off until a tag enables it. `scala_test` needs the tag matching the repository's declared test framework — `scala_deps.scalatest()`, `scala_deps.junit()`, or `scala_deps.specs2()` — and without it the target fails toolchain resolution before any test runs, with `No matching toolchains found for types: @@rules_scala+//testing/toolchain:testing_toolchain_type`.

`rules_scala` pins one patch release per Scala minor line — 7.2.6 ships 2.13.18 — and validates the configured version against it, so a config naming any other patch fails analysis with `Scala config (2.13.6) version does not match repository version (2.13.18)`. Keep the repository's declared version and supply its artifacts instead of lowering the declaration:

```starlark
scala_deps.overridden_artifact(
    name = "io_bazel_rules_scala_scala_library",
    artifact = "org.scala-lang:scala-library:2.13.6",
    sha256 = "<sha256>",
)
```

Repeat for `io_bazel_rules_scala_scala_compiler` and `io_bazel_rules_scala_scala_reflect`; `sha256` is mandatory on each. Do not reach for `scala_deps.settings(validate_scala_version = False)`: it silences the check while still compiling against the pinned jars, which is the failure the check exists to report.

Pin the JDK explicitly; local Java auto-detection fails in most CI and agent environments, with `Cannot find Java binary bin/java`. Use the current LTS unless the repository requires otherwise:

```text
common --java_runtime_version=remotejdk_25
common --tool_java_runtime_version=remotejdk_25
```

Older Scala 2.12 and 2.13 patch releases cannot parse class files from recent JDKs and fail inside `ClassfileParser` rather than with a version error. Where that happens, raise the Scala patch release — which the `overridden_artifact` path above already makes explicit — and treat lowering the JDK as a reported fallback, not the default. Carry sbt's `javacOptions` `-source`/`-target` across as `javacopts` on the targets owning Java sources.

For application dependencies, first preserve the selected sbt/Maven repository policy, evictions, overrides, exclusions, and lock or checksum behaviour. Introduce a pinned `rules_jvm_external` Maven installation only after those inputs are understood; use its generated versionless labels as direct dependencies and commit its lockfile. A project that relies on private repositories or a custom resolver must keep that authentication out of `MODULE.bazel`, source control, and the lockfile.

Use `scala_library`, `scala_binary`, and `scala_test` for hand-authored representative targets. Model macros, compiler plugins, test frameworks, resource directories, protobuf or Thrift generation, Java interop, and runtime data explicitly. Keep cross-built Scala variants as separate, named target sets until the repository has an intentional platform or configuration model; one accidental default toolchain is not a migration strategy.
