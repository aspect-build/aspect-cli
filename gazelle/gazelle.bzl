"""Utils for writing aspect-cli gazelle plugins"""

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

    # Use a genrule to rename .test-* files to .*
    dot_files = native.glob(["%s/**/.test-*" % dir], allow_empty = True)
    for i in range(len(dot_files)):
        dot_file = dot_files[i]
        native.genrule(
            name = "_%s-rename-%d" % (name, i),
            srcs = [dot_file],
            outs = [dot_file.replace(".test-", ".")],
            cmd = "cp $(location %s) \"$@\"" % dot_file,
        )
        data = data + [":_%s-rename-%d" % (name, i)]

    # Ensure a WORKSPACE exists, include all files in the test directory, exclude the renamed dot-files.
    data = data + native.glob(["%s/WORKSPACE" % dir, "%s/**" % dir], exclude = dot_files)

    _gazelle_generation_test(
        name = name,
        size = kwargs.pop("size", "small"),
        gazelle_binary = gazelle_binary,
        test_data = data,
        env = {"ASPECT_CLI_LOG_DEBUG": "trace"} | kwargs.pop("env", {}),
        **kwargs
    )
