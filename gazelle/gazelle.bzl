"""Utils for writing aspect-cli gazelle plugins"""

load("@aspect_bazel_lib//lib:copy_to_directory.bzl", "copy_to_directory")
load("@bazel_gazelle//:def.bzl", _gazelle_generation_test = "gazelle_generation_test")

def gazelle_generation_test(name, gazelle_binary, dir, data = [], **kwargs):
    """Creates a gazelle_generation_test target while supporting files such as ".gitignore" by prefixing with ".test-*"

    Args:
        name: name of the test
        gazelle_binary: gazelle binary target being tested
        dir: directory containing the test data
        data: additional data dependencies
        **kwargs: additional arguments to pass to gazelle_generation_test
    """

    # Rest of the function code...
    # Use copy_to_directory(replace_prefixes) to rename .test-* files to .*
    dot_files_replace_prefixes = {}
    for dot_file in native.glob(["%s/**/.test-*" % dir], allow_empty = True):
        dot_files_replace_prefixes[dot_file] = dot_file.replace(".test-", ".")

    # Data for each generation test.
    # Support files such as ".gitignore" by prefixing with ".test-*" and renaming at compile-time.
    # Copy to a target specific directory to support multiple gazelle_generation_test targets with the same dir.
    copy_to_directory(
        name = "_%s-data" % name,
        out = "%s_test" % name,
        replace_prefixes = dot_files_replace_prefixes,
        srcs = native.glob(["%s/**" % dir]),
        testonly = True,
        visibility = ["//visibility:private"],
    )

    _gazelle_generation_test(
        name = name,
        size = kwargs.pop("size", "small"),
        gazelle_binary = gazelle_binary,
        test_data = data + [":_%s-data" % name],
        **kwargs
    )
