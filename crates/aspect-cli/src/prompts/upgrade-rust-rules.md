# Upgrade this Rust Bazel repository to rules_rs

This is an incremental migration from a legacy `rules_rust` and crate-universe setup to `rules_rs`, not a fresh Bazelification. Inspect the existing Bazel module or workspace files, `Cargo.toml`, `Cargo.lock`, toolchains, crate-universe declarations, BUILD layout, generated code, tests, and CI platforms before changing anything. Preserve the repository's Bazel major and working target labels during the first migration stage.

Migrate one representative crate at a time using this mapping:

| Existing setup | rules_rs replacement | Validate before retiring it |
| --- | --- | --- |
| `rules_rust` dependency and direct `@rules_rust` loads | `rules_rs` plus its `rules_rust` compatibility facade | Existing library, binary, and test labels still build. |
| `rust_register_toolchains` and legacy Rust toolchain repositories | The `rules_rs` `toolchains` module extension and registered toolchains | Rust edition, compiler version, execution platforms, and targets are unchanged. |
| `crate_universe`, `crate.spec`, or generated crate repository | The `crate.from_cargo` extension over the committed `Cargo.toml` and `Cargo.lock` | A representative crate resolves normal, development, optional, and platform-specific dependencies. |
| Hand-maintained third-party crate labels | The generated `@crates` hub accessors | Direct dependencies and feature selections resolve without broad aliases. |

Add `rules_rs` and its compatibility facade beside the legacy setup first. Move a representative crate's dependency resolution to `crate.from_cargo` while retaining its existing `@rules_rust` target loads through the facade, then build and test it with the current Cargo commands alongside Bazel. Move toolchain registration only after that crate selects the same edition, compiler, and platforms. Migrate the remaining crates package by package; do not rename targets or collapse BUILD directories merely to make hub labels resolve.

Remove crate-universe declarations, legacy toolchains, and `rules_rust` only after `bazel mod tidy`, representative binaries and integration tests, and the relevant CI platform matrix pass. Report any Cargo manifest, lockfile, feature, generated-source, or test-harness change that the new dependency model requires; do not make a packaging or build-semantic change silently.
