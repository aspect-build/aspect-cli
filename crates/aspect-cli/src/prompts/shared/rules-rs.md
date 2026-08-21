## rules_rs Paved Path

Use `rules_rs` `0.0.102`. Derive Rust edition and minimum compiler version from the repository; do not lower the toolchain below its declared `rust-version`.

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

Identify private Git dependencies during lockfile resolution. If they cannot be fetched reproducibly in Bazel and CI, exclude the affected independent workspace explicitly and report the authentication or repository arrangement it requires; a private source does not by itself require a separate migration.

A `-sys` crate whose build script probes the host for a native library does not build hermetically. Annotate it once a selected target demonstrates the need:

```starlark
crate.annotation(
    crate = "pq-sys",
    gen_build_script = "off",
    deps = ["@postgres//:libpq"],
)
inject_repo(crate, "postgres")
```

Either turn the generated build script off and point the crate at a library another module provides, or keep the script and supply what it probes for through `build_script_env` and `build_script_data`. `inject_repo` makes a `bazel_dep` repository visible to the extension. Where a crate's binaries are needed by a toolchain rather than by a target, expose them with `gen_binaries`.

`rules_rs` prints `WARNING: A well-known crate annotation exists for <crate>!` whenever a crate's build script is still `auto` and a snippet exists at `3rd_party/<crate>/include.MODULE.bazel`. Those snippets are starting points, not evidence that the annotation preserves the crate's Cargo semantics for the repository's target platform. The shipped `tikv-jemalloc-sys` snippet disables the build script without restoring the prefixed allocator ABI that Apple targets require.

Turning a generated build script off transfers its relevant contract to the Bazel replacement, not merely its choice of native library. Inspect both the build-script source and its actual Cargo output for `rustc-cfg`, `rustc-env`, linker directives, generated files, target-dependent behaviour, and native configuration options. Model target-dependent configuration explicitly with facilities such as `rustc_flags_select`.

In `rules_rs` `0.0.102`, `crate.annotation` does not expose `rustc_env`. If disabling a build script removes required `cargo::rustc-env` output, keep the script or provide another explicit, validated mechanism.

Native-library ABI choices, including symbol prefixes, private namespaces, and feature toggles, belong on the target providing that library. Compare the crate's declared external symbols with those present in the linked binary. For global allocators, verify the intended symbol namespace and execute a representative binary; successful analysis, compilation, and linking are insufficient.

Build scripts using compile-time `env!("CARGO_MANIFEST_DIR")` or `env!("OUT_DIR")` can embed paths belonging to their compilation action rather than their execution action. Compare the embedded path, staged inputs, and execution sandbox before deciding whether the remedy belongs in the crate, its annotation, or `rules_rs`. Adding `build_script_data` does not repair a stale embedded path when the required file is already staged for the execution action.

For an existing `@rules_rust` build, retain its loads temporarily through the compatibility facade:

```starlark
rules_rust = use_extension("@rules_rs//rs:rules_rust.bzl", "rules_rust")
use_repo(rules_rust, "rules_rust")
```

Cargo targets, not source directories, are the unit of migration. Enumerate the library, binaries, integration tests, and CI-built examples with `cargo metadata`; a Rust module is never a separate Bazel target. Keep the selected targets in the Cargo package directory's BUILD file. Do not glob binary roots from `src/main.rs` or `src/bin/` into a library, or every integration-test root from `tests/` into one test target.

`rules_rs` publishes one file per rule and no `defs.bzl`:

```starlark
load("@rules_rs//rs:rust_library.bzl", "rust_library")
load("@rules_rs//rs:rust_test.bzl", "rust_test")

filegroup(
    name = "migrations",
    srcs = glob(["src/migrations/**/*.sql"]),
)

rust_library(
    name = "event-queue",
    srcs = glob(["src/**/*.rs"]),
    compile_data = [":migrations"],
    crate_features = ["default", "serde"],
    crate_name = "event_queue",
    deps = [
        "//crates/matcher",
        "@crates//:tokio",
    ] + select({
        "@platforms//os:windows": ["@crates//:winapi-util"],
        "//conditions:default": ["@crates//:libc"],
    }),
)

rust_test(
    name = "archive_cursor_test",
    srcs = ["tests/archive_cursor.rs"],
    crate_root = "tests/archive_cursor.rs",
    deps = [
        ":event-queue",
        "@crates//:tempfile",
    ],
)
```

The target name follows the Cargo package name; `crate_name` is that name's Rust spelling, with dashes replaced by underscores. Third-party dependencies are `@<hub>//:<crate>` under the name given to `crate.from_cargo`. First-party workspace members also appear in that hub; depending on them there builds a second copy instead of the in-repo target. Each integration-test root takes its own `rust_test` with an explicit `crate_root`. A manifest rename such as `memmap = { package = "memmap2" }` needs `aliases = {"@crates//:memmap2": "memmap"}`, or the crate is unresolvable under the name its callers import. Read dependencies from `cargo metadata` rather than by eye: `[dependencies.<name>]` tables and `[target.'cfg(...)'.dependencies]` are easy to miss, and the latter is what the `select` above models.

The generated hub does not infer first-party feature selections. Derive each target's features from the package manifest and resolved Cargo feature graph; a generic `default` feature list can compile while changing behaviour.

Use `compile_data` for `include_str!` and `include_bytes!`, and `data` for runtime runfiles. Integration tests need both normal and development dependencies. A test that spawns a Bazel binary must resolve its `$(rlocationpath :binary)` from runfiles at runtime; `CARGO_BIN_EXE_<name>` is compile-time-only. Confirm every test log reports a non-zero test count.

Manually declared first-party Rust targets do not automatically receive Cargo's `CARGO_PKG_*` variables. Supply required values through `rustc_env`, derive them from the authoritative manifest, and record that derivation so a copied literal is not mistaken for an independently chosen value.
