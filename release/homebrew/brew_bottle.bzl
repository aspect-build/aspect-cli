"""Macro for generating a Homebrew bottle

Implementation for `brew bottle`: 
https://github.com/Homebrew/brew/blob/8dc46a7c477929185cba8ca0de5f9c843b3e9385/Library/Homebrew/dev-cmd/bottle.rb#L288

Example bottle layout for `bazelisk` version 1.14.0:
```
bazelisk
└── 1.14.0
    ├── LICENSE
    ├── README.md
    ├── bin
    │   ├── bazel -> bazelisk
    │   └── bazelisk
    └── share
        └── zsh
            └── site-functions
                └── _bazel
```

"""

load("@aspect_bazel_lib//lib:utils.bzl", "to_label")
load("@bazel_skylib//rules:build_test.bzl", "build_test")
load("@rules_pkg//pkg:mappings.bzl", "pkg_attributes", "pkg_files")
load("@rules_pkg//pkg:tar.bzl", "pkg_tar")

def brew_bottle(
        name,
        formula,
        version_file,
        root_files = None,
        bin_files = None,
        bin_renames = None,
        visibility = None,
        testonly = False):
    """Define rules for generating a Homebrew bottle and related artifacts.

    Args:
        name: The name for the bottle target as a `string`.
        formula: The name of the Homebrew formula as a `string`.
        version_file: The label referencing a version file as written by `version_file`.
        root_files: Optional. A `sequence` of files to be placed in the root of
            the bottle.
        bin_files: Optional. A `sequence` of files to be placed in the
            formula's bin directory.
        bin_renames: Optional. A `dict` of `Label -> string` that maps files
            that should be renamed before being placed in the `bin` directory.
        visibility: Optional. A `list` of visibility declarations that should
            be applied to the output targets.
        testonly: Optional. A `bool` specifying whether the targets defined by
            this macro are for test only.
    """

    srcs = []

    version_label = to_label(version_file)
    package_dir_file_name = "{}_dir_file".format(name)
    native.genrule(
        name = package_dir_file_name,
        outs = ["{}.dir_file".format(name)],
        srcs = [version_label],
        cmd = """\
    formula="{formula}"
    version_file=$(location {version_label})
    """.format(
            formula = formula,
            version_label = version_label,
        ) + """\
    version="$$(< "$${version_file}")"
    echo "$${formula}/$${version}" > $@
    """,
        testonly = testonly,
    )

    if root_files:
        root_files_name = "{}_root_files".format(name)
        pkg_files(
            name = root_files_name,
            srcs = root_files,
            testonly = testonly,
        )
        srcs.append(root_files_name)

    if bin_files:
        bin_files_name = "{}_bin_files".format(name)
        pkg_files(
            name = bin_files_name,
            srcs = bin_files,
            renames = bin_renames,
            prefix = "bin",
            testonly = testonly,
            attributes = pkg_attributes(mode = "0555"),
        )
        srcs.append(bin_files_name)

    pkg_tar(
        name = name,
        srcs = srcs,
        extension = "tar.gz",
        visibility = visibility,
        testonly = testonly,
        package_dir_file = package_dir_file_name,
    )

    build_test(
        name = "{}_build_test".format(name),
        targets = [name],
    )
