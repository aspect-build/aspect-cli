"""Tests for `brews` Starlark module."""

load("@bazel_skylib//lib:unittest.bzl", "asserts", "unittest")
load("//bazel/release/homebrew:brews.bzl", "brews")

def _bottle_name_test(ctx):
    env = unittest.begin(ctx)

    actual = brews.bottle_name("monterey")
    asserts.equals(env, "monterey_bottle", actual)

    actual = brews.bottle_name("foo", "monterey")
    asserts.equals(env, "foo_monterey_bottle", actual)

    actual = brews.bottle_name("foo", "arm64_monterey")
    asserts.equals(env, "foo_arm64_monterey_bottle", actual)

    return unittest.end(env)

bottle_name_test = unittest.make(_bottle_name_test)

def _ruby_class_name_test(ctx):
    env = unittest.begin(ctx)

    actual = brews.ruby_class_name("foo")
    asserts.equals(env, "Foo", actual)

    actual = brews.ruby_class_name("foo-bar")
    asserts.equals(env, "FooBar", actual)

    actual = brews.ruby_class_name("foo_bar")
    asserts.equals(env, "FooBar", actual)

    actual = brews.ruby_class_name("foo.bar")
    asserts.equals(env, "FooBar", actual)

    actual = brews.ruby_class_name("foo_bar-chicken.hello")
    asserts.equals(env, "FooBarChickenHello", actual)

    return unittest.end(env)

ruby_class_name_test = unittest.make(_ruby_class_name_test)

def brews_test_suite():
    return unittest.suite(
        "brews_tests",
        bottle_name_test,
        ruby_class_name_test,
    )
