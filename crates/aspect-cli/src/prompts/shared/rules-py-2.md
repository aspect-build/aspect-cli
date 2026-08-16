## Aspect rules_py 2.x recipe

Use `aspect_rules_py` `2.0.0-alpha.4` only after checking it is compatible with the repository's Bazel version. The Bazel-managed `uv` toolchain does not require host `uv`; refresh the lock from the workspace rather than Bazel's runfiles tree:

```text
bazel run @uv//:uv -- lock --project $(bazel info workspace)
```

```starlark
bazel_dep(name = "aspect_rules_py", version = "2.0.0-alpha.4")

interpreters = use_extension(
    "@aspect_rules_py//py:extensions.bzl",
    "python_interpreters",
)
interpreters.toolchain(python_version = "<selected-version>")
use_repo(interpreters, "python_interpreters")
register_toolchains("@python_interpreters//:all")

uv_bin = use_extension("@aspect_rules_py//uv:extensions.bzl", "uv_bin")
uv_bin.toolchain(version = "0.11.6")
use_repo(uv_bin, "uv")
register_toolchains("@uv//:all")

uv = use_extension("@aspect_rules_py//uv:extensions.bzl", "uv")
uv.declare_hub(hub_name = "pypi")
uv.project(
    hub_name = "pypi",
    pyproject = "//:pyproject.toml",
    lock = "//:uv.lock",
)
use_repo(uv, "pypi")
```

Use `py_library`, `py_binary`, and `py_test` from `@aspect_rules_py//py:defs.bzl`; pytest uses `py_test(pytest_main = True)`. Put `python_version` on the first `py_binary` and `py_test`, not `py_library`. Hub labels are underscore-normalised, for example `@pypi//zope_interface` rather than PEP 503 hyphen spelling.

A source-only `py_library` can report `nothing to build`; prove a first slice with a small binary or test that imports it. `uv lock` resolves with the interpreter it finds, not necessarily Bazel's selected interpreter, so check the lock's Python requirement and markers.

This recipe cannot consume a `uv.lock` whose root project omits `version`, including a project using `dynamic = ["version"]`; the hub fails with `key "version" not found in dictionary`. A static `project.version` is the only workaround, so obtain agreement before changing versioning semantics.

When `[dependency-groups]` exist, they replace the implicit project group. Declare the runtime and test groups Bazel needs, use `dep_group` on consuming targets, and refresh through `@uv` with `--noworkspace_rc` after setting a workspace-wide default.
