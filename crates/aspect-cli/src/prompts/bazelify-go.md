# Bazel-ify this Go repository

Inspect the repository's `go.mod` and `go.work` files, module layout, generated sources, cgo use, test conventions, and any replace or vendor directives before changing build files. Preserve the useful Go module workflow while introducing Bazel incrementally; do not replace a working build wholesale.

Use Bzlmod with `rules_go` and Gazelle. Start with one representative package, binary, or test target, then make module dependencies, Go toolchain selection, generated code, cgo constraints, and platform-specific behaviour explicit. Keep dependency resolution reproducible and avoid a broad conversion that masks module boundaries or generated inputs.

Treat checked-in `.pb.go` files as ordinary Go sources in the owning `go_library`; do not introduce `protoc`, `rules_proto`, or generated-code targets merely because those files exist. If protobuf sources themselves need Bazel ownership, use the separate `bazelify-protos` prompt. A Go target consuming Bazel-generated protobuf code should depend on that package's generated Go target, rather than compiling generated files a second way.

Establish Gazelle policy before generating BUILD files. Use `# gazelle:prefix` for the Go module path; add `# gazelle:exclude <directory>` only for a subtree Gazelle actually generates into that `go build ./...` ignores, verified by running it — adding directives pre-emptively for dot-directories or Go-free trees Gazelle already skips is noise. Where checked-in `.pb.go` is the source of truth across the repository, add `# gazelle:proto disable_global` in the root BUILD file and apply the same default to external Go modules:

```starlark
go_deps.gazelle_default_attributes(
    directives = ["gazelle:proto disable_global"],
)
```

If only one local proto package uses checked-in generated code, use `# gazelle:proto disable` in that package instead, and a `go_deps.gazelle_override` only for an external module with the same policy. Adding a directive does not remove rules Gazelle already generated. Before regenerating, identify generated-only BUILD files affected by the policy, preserve any manual rules, then delete and regenerate just those files.

Gazelle recognises `testdata/`, not arbitrary fixtures. Declare other fixture directories in `data` explicitly. Expose corpora shared between Bazel packages through a visible `filegroup`; do not glob a child directory that has its own BUILD file from its parent package. Hand-editing a Gazelle-generated target is permitted for attributes Gazelle does not own, such as explicit fixture `data`; preserve that intent on later generated diffs.

After a representative build and test work under Bazel, report whether the repository is ready for the separate remote-configuration prompt. Do not configure remote execution, caching, or build-event streaming as part of this migration.

Validate each stage with the repository's existing `go test` and build commands alongside the equivalent Bazel targets. Report unresolved differences rather than hiding them behind broad tags or local-only configuration.

`go test` in a `go.work` workspace may resolve unrelated modules before the selected package runs. Distinguish that dependency-resolution failure from a package test failure. Do not expand the initial Bazel slice merely to make an unavailable network or module cache succeed; use an approved cache location when available, validate the local build where possible, and report the unavailable test prerequisite. Inventory checked-in protobuf sources before applying `disable_global`; use the global policy only when it is the repository-wide source-of-truth decision.
