# Bazel-ify these Protocol Buffer sources

Inspect every `.proto` package, its imports, checked-in generated files, compiler/plugin configuration, language consumers, and compatibility expectations before changing build files. Decide the source of truth per proto package; do not make a repository-wide choice merely because one directory happens to contain generated Go, Python, or Rust files.

For a package with checked-in generated sources, keep those sources as the inputs to its language-specific library target. Do not add a `proto_library`, compiler toolchain, or generation step only to reproduce files that another workflow already owns. Scope any Gazelle protobuf directive to the affected package; never add `# gazelle:proto disable_global` at the repository root merely to quiet a conversion.

For a package that Bazel should generate, make the boundary explicit: define a `proto_library` owning only the package's `.proto` files and direct imports, then create one generated-language target for each consumer language. In Go, a `go_proto_library` wraps that `proto_library`, and Go binaries, libraries, and tests depend on the `go_proto_library` target. They do not compile its generated `.pb.go` outputs separately. Follow the equivalent ruleset convention for other languages.

Migrate one proto package and one consumer at a time. A package must have exactly one active path for each language: checked-in generated sources or Bazel-generated sources. Validate wire compatibility, import paths, generated package/module names, and a representative consumer build and test before migrating another package. Report any required regeneration or checked-in-file removal rather than mixing both paths to obtain a green build.
