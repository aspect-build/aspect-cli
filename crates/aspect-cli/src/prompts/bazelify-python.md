# Bazel-ify this Python repository

Identify the actual Python packaging unit, its `pyproject.toml`, lockfile, interpreter range, test runner, generated sources, and native extensions. Do not assume the repository root is the package or that one lockfile covers every unit.

Use the bundled `rules_py` 2.x recipe. Before enabling a lockfile's `uv.project`, inspect it for editable or path sources. Each sibling editable source needs an explicit Bazel target mapping; the root project itself does not.

```starlark
uv.override_package(
    lock = "//<package>:uv.lock",
    name = "<editable-package-name>",
    target = "//<editable-package>:package",
)
```

Start with a small library, binary, or test whose dependency closure is meaningful. Make dependency groups, entry points, generated code, native extensions, and platform constraints explicit. Keep source lists local to the package; do not use a recursive `glob(["**/*.py"])` as the durable layout.

Keep `pyproject.toml` and `uv.lock` authoritative. Report any required change to published package semantics, dependency intent, or versioning policy rather than making it silently.
