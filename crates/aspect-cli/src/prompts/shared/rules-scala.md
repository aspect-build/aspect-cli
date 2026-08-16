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
```

For application dependencies, first preserve the selected sbt/Maven repository policy, evictions, overrides, exclusions, and lock or checksum behaviour. Introduce a pinned `rules_jvm_external` Maven installation only after those inputs are understood; use its generated versionless labels as direct dependencies and commit its lockfile. A project that relies on private repositories or a custom resolver must keep that authentication out of `MODULE.bazel`, source control, and the lockfile.

Use `scala_library`, `scala_binary`, and `scala_test` for hand-authored representative targets. Model macros, compiler plugins, test frameworks, resource directories, protobuf or Thrift generation, Java interop, and runtime data explicitly. Keep cross-built Scala variants as separate, named target sets until the repository has an intentional platform or configuration model; one accidental default toolchain is not a migration strategy.
