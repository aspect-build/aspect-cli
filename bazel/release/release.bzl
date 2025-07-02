"""This module provides the macros for performing a release.
"""

def release(name, targets, **kwargs):
    """The release macro creates the artifact copier script.

    It's an executable script that copies all artifacts produced by the given
    targets into the provided destination. See .github/workflows/release.yml.

    Args:
        name: the name of the genrule.
        targets: a list of filegroups passed to the artifact copier.
        **kwargs: extra arguments.
    """
    native.genrule(
        name = name,
        srcs = targets,
        outs = [name + ".sh"],
        executable = True,
        cmd = "./$(location //bazel/release:create_release.sh) {locations} > \"$@\"".format(
            locations = " ".join(["$(locations {})".format(target) for target in targets]),
        ),
        tools = ["//bazel/release:create_release.sh"],
        tags = kwargs.pop("tags", []) + ["manual"],
        **kwargs
    )
