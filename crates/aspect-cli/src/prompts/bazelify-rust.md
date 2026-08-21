# Bazel-ify this Rust repository

Use the bundled `rules_rs` Paved Path with the repository's `Cargo.toml` and `Cargo.lock`; do not create a second Bazel-specific Cargo lockfile. Establish the foundation on one meaningful library, binary, or test, then migrate the remaining workspace members in stages.

Make build scripts, proc macros, generated sources, native dependencies, feature variants, and platform constraints explicit. When cross-compiling, distinguish execution platforms for build scripts and proc macros from target platforms for the produced binary.

Keep Cargo manifests and lockfiles authoritative. Use the temporary `rules_rs` compatibility export only while existing `@rules_rust` loads are being migrated, then move representative targets to `@rules_rs//rs:*` before removing it.
