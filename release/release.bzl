"""This module provides the macros for performing a release.
"""

load("@aspect_bazel_lib//lib:transitions.bzl", "platform_transition_filegroup")
load("@io_bazel_rules_go//go:def.bzl", "go_binary")
load(":hashes.bzl", "hashes")
load(":platforms.bzl", "platforms")

def multi_platform_binaries(name, embed, prefix = "", **kwargs):
    """The multi_platform_binaries macro creates a go_binary for each platform.

    Args:
        name: the name of the filegroup containing all go_binary targets produced
            by this macro.
        embed: the list of targets passed to each go_binary target in this
            macro.
        prefix: an optional prefix added to the output Go binary file name.
        **kwargs: extra arguments.
    """
    go_binary(
        name = "_{}".format(name),
        # NB: This rule is used to create Aspect CLI releases and Aspect CLI plugin releases.
        # The Aspect CLI assumes that the platforms names are `<os>_arm64` and `<os>_amd64`.
        # This naming convention cannot be changed for releases without it being a BREAKING CHANGE.
        out = select({
            "//platforms/config:linux_aarch64": "{}{}-linux_arm64".format(prefix, name),
            "//platforms/config:linux_x86_64": "{}{}-linux_amd64".format(prefix, name),
            "//platforms/config:macos_aarch64": "{}{}-darwin_arm64".format(prefix, name),
            "//platforms/config:macos_x86_64": "{}{}-darwin_amd64".format(prefix, name),
        }),
        gc_linkopts = ["-s", "-w"],
        embed = embed,
        cgo = True,
        visibility = ["//visibility:public"],
        **kwargs
    )
    targets = []
    for platform in platforms.all:
        target_name = platforms.go_binary_target_name(name, platform)
        target_label = Label("//{}:{}".format(native.package_name(), target_name))
        platform_transition_filegroup(
            name = target_name,
            srcs = [":_{}".format(name)],
            target_platform = "//platforms:{}_{}".format(platform.os, platform.arch),
            **kwargs
        )
        hashes_name = "{}_hashes".format(target_name)
        hashes_label = Label("//{}:{}".format(native.package_name(), hashes_name))
        hashes(
            name = hashes_name,
            src = target_label,
            **kwargs
        )
        targets.extend([target_label, hashes_label])

    native.filegroup(
        name = name,
        srcs = targets,
        **kwargs
    )

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
        outs = ["release.sh"],
        executable = True,
        cmd = "./$(location //release:create_release.sh) {locations} > \"$@\"".format(
            locations = " ".join(["$(locations {})".format(target) for target in targets]),
        ),
        tools = ["//release:create_release.sh"],
        **kwargs
    )
