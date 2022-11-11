"""Macro to generate a version file"""

# We can't use the bazel-lib one, because it doesn't have a program to read stamp vars.
# see https://github.com/aspect-build/rules_js/pull/384#issue-1337742941
# load("@aspect_bazel_lib//lib:expand_make_vars.bzl", "expand_template")
# buildifier: disable=bzl-visibility
load("@aspect_rules_js//js/private:expand_template.bzl", "expand_template")

def version_file(name, version_var, **kwargs):
    """Generate a file that contains the semver stored in the specified \
    workspace status variable.

    Args:
        name: The name of the target.
        version_var: The name of the workspace status variable.
        **kwargs: Other attributes passsed to underlying rules.
    """
    expand_template(
        name = name,
        out = "{}.version".format(name),
        stamp_substitutions = {
            "0.0.0-VERSION-PLACEHOLDER": "{{" + version_var + "}}",
        },
        template = "//release:version.tmpl",
        **kwargs
    )
