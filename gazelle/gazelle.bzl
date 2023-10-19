"""Utils for writing aspect-cli gazelle plugins"""

load("@bazel_gazelle//:def.bzl", _gazelle_generation_test = "gazelle_generation_test")

def gazelle_generation_test(name, gazelle_binary, dir, **kwargs):
    _gazelle_generation_test(
        name = name,
        size = kwargs.pop("size", "small"),
        gazelle_binary = gazelle_binary,
        test_data = [
            ":_%s-data" % name,
        ],
        **kwargs
    )

    # Data for each generation test.
    # Support files such as ".gitignore" by prefixing with ".test-*" and rename as a genrule
    # when declaring the filegroup of test data.
    native.filegroup(
        name = "_%s-data" % name,
        srcs = native.glob(
            ["%s/**" % dir],
            exclude = ["%s/**/.test-*" % dir],
        ) + [s.replace(".test-", ".") for s in native.glob(["%s/**/.test-*" % dir], allow_empty = True)],
        visibility = ["//visibility:private"],
    )

    for s in native.glob(["%s/**/.test-*" % dir], allow_empty = True):
        native.genrule(
            name = s.replace("/", "_").replace(".", "_"),
            srcs = [s],
            outs = [s.replace(".test-", ".")],
            cmd = "cat $(location %s) > $@" % s,
            visibility = ["//visibility:private"],
        )
