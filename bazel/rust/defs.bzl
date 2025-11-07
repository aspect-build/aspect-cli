"""Rust rule defaults"""

load("@aspect_bazel_lib//lib:expand_template.bzl", _expand_template = "expand_template")
load("@rules_rust//rust:defs.bzl", _rust_binary = "rust_binary", _rust_library = "rust_library", _rust_proc_macro = "rust_proc_macro", _rust_test = "rust_test")

_default_platform = select({
    # Force Linux binaries to be built with musl
    "//bazel/platforms/config:linux_aarch64": "//bazel/platforms:linux_aarch64_musl",
    "//bazel/platforms/config:linux_x86_64": "//bazel/platforms:linux_x86_64_musl",
    # Non-Linux binaries should just build with their default platforms
    "//conditions:default": None,
})

def rust_binary(name, rustc_env_files = [], version_key = "", crate_features = [], **kwargs):
    """
    Macro for rust_binary defaults.

    Args:
        name: Name of the rust_binary target
        rustc_env_files: Additional env files to pass to the rust compiler
        version_key: Stamp key to use for version replacement at compile time
        crate_features: Create features to enable for the binary target
        **kwargs: Additional args to pass to rust_binary
    """

    if version_key != None:
        rustc_env_file = "{}_rustc_env_file".format(name)
        _expand_template(
            name = "{}_env_file".format(name),
            out = rustc_env_file,
            stamp_substitutions = {"0.0.0-dev": "{{%s}}" % (version_key)},
            template = [
                "CARGO_PKG_VERSION=0.0.0-dev",
            ],
        )
        rustc_env_files = rustc_env_files + [rustc_env_file]

    platform = kwargs.pop("platform", _default_platform)

    _rust_binary(
        name = name,
        rustc_env_files = rustc_env_files,
        rustc_flags = select({
            "//bazel/release:is_opt": [
                "-Ccodegen-units=1",
                "-Copt-level=3",
                "-Cpanic=abort",
                "-Cstrip=symbols",
            ],
            "//conditions:default": [
                "-Ccodegen-units=1",
                "-Copt-level=3",
            ],
        }),
        crate_features = crate_features + ["bazel"],
        platform = platform,
        **kwargs
    )

def rust_test(name, crate_features = [], **kwargs):
    platform = kwargs.pop("platform", _default_platform)

    _rust_test(
        name = name,
        crate_features = crate_features + ["bazel"],
        platform = platform,
        rustc_flags = select({
            "//conditions:default": [
                "-Copt-level=0",
            ],
        }),
        **kwargs
    )

def rust_library(name, rustc_env_files = [], version_key = "", crate_features = [], **kwargs):
    """
    Macro for rust_library defaults.

    Args:
        name: Name of the rust_library target
        rustc_env_files: Additional env files to pass to the rust compiler
        version_key: Stamp key to use for version replacement at compile time
        crate_features: Create features to enable for the library target
        **kwargs: Additional args to pass to rust_library
    """
    stamp = 0
    if version_key != None:
        rustc_env_file = "{}_rustc_env_file".format(name)
        _expand_template(
            name = "{}_env_file".format(name),
            out = rustc_env_file,
            stamp_substitutions = {"0.0.0-dev": "{{%s}}" % (version_key)},
            template = [
                "CARGO_PKG_VERSION=0.0.0-dev",
            ],
        )
        stamp = -1 # workaround https://github.com/bazelbuild/rules_rust/pull/3503
        rustc_env_files = rustc_env_files + [rustc_env_file]
    _rust_library(
        name = name,
        rustc_env_files = rustc_env_files,
        rustc_flags = select({
            "//bazel/release:is_opt": [
                "-Ccodegen-units=1",
                "-Copt-level=3",
                "-Cpanic=abort",
                "-Cstrip=symbols",
            ],
            "//conditions:default": [
                "-Ccodegen-units=1",
                "-Copt-level=3",
            ],
        }),
        crate_features = crate_features + ["bazel"],
        stamp = stamp,
        **kwargs
    )

def rust_proc_macro(name, crate_features = [], **kwargs):
    """
    Macro for rust_proc_macro defaults.

    Args:
        name: Name of the rust_proc_macro target
        crate_features: Create features to enable for the target
        **kwargs: Additional args to pass to rust_proc_macro
    """
    _rust_proc_macro(
        name = name,
        rustc_flags = select({
            "//bazel/release:is_opt": [
                "-Ccodegen-units=1",
                "-Copt-level=3",
                "-Cpanic=abort",
                "-Cstrip=symbols",
            ],
            "//conditions:default": [
                "-Ccodegen-units=1",
                "-Copt-level=3",
            ],
        }),
        crate_features = crate_features + ["bazel"],
        **kwargs
    )
