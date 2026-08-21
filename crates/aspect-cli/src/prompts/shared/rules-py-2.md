## Aspect rules_py 2.x recipe

Use `aspect_rules_py` `2.0.0-alpha.4`. The Bazel-managed `uv` toolchain does not require host `uv`; refresh the lock from the workspace rather than Bazel's runfiles tree:

```text
bazel run @uv//:uv -- lock --project $(bazel info workspace)
```

A repository with no `uv.lock` at all needs one before the hub can read it. Where the requirements live in `requirements.txt` rather than `pyproject.toml`, seed the project from them and report the new files as a source change:

```text
bazel run @uv//:uv -- init --no-workspace
bazel run @uv//:uv -- add -r requirements_lock.txt
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

The hub serves one dependency group at a time, selected by a flag on the hub itself. Each `[dependency-groups]` entry in `pyproject.toml` registers as a named group; with no such entries the hub creates one implicit group named after the project, verbatim, so a `name = "Scrapy"` project yields the group `Scrapy` even though its labels are lowercase. Set the workspace default in `.bazelrc` and override it per target:

```text
common --@pypi//dep_group=test
```

```starlark
load("@aspect_rules_py//py:defs.bzl", "py_test")
load("@pypi//:defs.bzl", "group_deps")

py_test(
    name = "unit",
    srcs = ["test_item.py"],
    dep_group = "test",
    pytest_main = True,
    python_version = "3.13",
    deps = ["@pypi//cowsay"],
)

py_test(
    name = "unit_all_deps",
    srcs = ["test_item.py"],
    dep_group = "test",
    pytest_main = True,
    deps = group_deps(),
)
```

`dep_group` exists on `py_binary`, `py_test`, and `py_venv`, not on `py_library`. `group_deps()` resolves to the consuming target's active group, so the group name is never repeated; `all_requirements` from `requirements.bzl` remains the hub-wide union and may contain targets incompatible with the selected group. Declaring any `[dependency-groups]` entry removes the implicit group and everything in it, so a project whose runtime dependencies live in `[project].dependencies` must mirror them into a group and include it, and report that duplication rather than making it silently:

```toml
[dependency-groups]
runtime = ["<copy of project.dependencies>"]
test = [{ include-group = "runtime" }, "pytest>=8.4.1"]
```

Refresh the lock through `@uv` with `--noworkspace_rc` once a workspace-wide default is set, so the flag does not leak into the uv toolchain's own build. To substitute a first-party target for a locked requirement, use `uv.override_package(lock, name, target)` rather than editing the lockfile.
