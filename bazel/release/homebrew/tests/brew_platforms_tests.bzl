"""Tests for `brew_platforms` module"""

load("@bazel_skylib//lib:unittest.bzl", "asserts", "unittest")
load("//bazel/release:platforms.bzl", "platforms")
load("//bazel/release/homebrew:brew_platforms.bzl", "brew_platforms")

def _new_test(ctx):
    env = unittest.begin(ctx)

    rust_platform = platforms.get(os = platforms.oss.MACOS, arch = platforms.archs.ARM64)
    actual = brew_platforms.new(brew_platforms.names.MONTEREY_ARM64, rust_platform)
    expected = struct(
        name = brew_platforms.names.MONTEREY_ARM64,
        rust_platform = rust_platform,
    )
    asserts.equals(env, expected, actual)

    return unittest.end(env)

new_test = unittest.make(_new_test)

def brew_platforms_test_suite():
    return unittest.suite(
        "brew_platforms_tests",
        new_test,
    )
