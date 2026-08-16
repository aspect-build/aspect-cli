## rules_rs Paved Path

Use `rules_rs` `0.0.102` after checking it is compatible with the repository's Bazel version. Derive Rust edition and minimum compiler version from the repository; do not lower the toolchain below its declared `rust-version`.

```starlark
bazel_dep(name = "rules_rs", version = "0.0.102")
bazel_dep(name = "llvm", version = "0.8.9")
bazel_dep(name = "platforms", version = "1.1.0")

toolchains = use_extension(
    "@rules_rs//rs/toolchains:module_extension.bzl",
    "toolchains",
)
toolchains.toolchain(
    edition = "<workspace-package-edition>",
    version = "<rust-version-from-cargo-metadata>",
)
use_repo(toolchains, "default_rust_toolchains")
register_toolchains(
    "@default_rust_toolchains//:all",
    "@llvm//toolchain:all",
)

crate = use_extension("@rules_rs//rs:extensions.bzl", "crate")
crate.from_cargo(
    name = "crates",
    cargo_lock = "//:Cargo.lock",
    cargo_toml = "//:Cargo.toml",
    platform_triples = ["<initial-host-triple>"],
)
use_repo(crate, "crates")
```

`crate.from_cargo` resolves the entire lockfile graph, including optional crates and platforms unrelated to the first target. Do not change global configuration to silence those failures until a selected target demonstrates the need.

For an existing `@rules_rust` build, retain its loads temporarily through the compatibility facade:

```starlark
rules_rust = use_extension("@rules_rs//rs:rules_rust.bzl", "rules_rust")
use_repo(rules_rust, "rules_rust")
```

Cargo targets, not source directories, are the unit of migration. Enumerate the library, binaries, integration tests, and CI-built examples with `cargo metadata`; a Rust module is never a separate Bazel target. Keep the selected targets in the Cargo package directory's BUILD file. Do not glob binary roots from `src/main.rs` or `src/bin/` into a library, or every integration-test root from `tests/` into one test target.

The generated hub does not infer first-party feature selections. Derive each target's features from the package manifest and resolved Cargo feature graph; a generic `default` feature list can compile while changing behaviour.

Use `compile_data` for `include_str!` and `include_bytes!`, and `data` for runtime runfiles. Integration tests need both normal and development dependencies. A test that spawns a Bazel binary must resolve its `$(rlocationpath :binary)` from runfiles at runtime; `CARGO_BIN_EXE_<name>` is compile-time-only. Confirm every test log reports a non-zero test count.
