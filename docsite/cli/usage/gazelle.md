


[Gazelle](https://github.com/bazel-contrib/bazel-gazelle) is the standard tool to generate or update Bazel build definitions (files named `BUILD`, which Bazel uses to describe build targets) based on the content of source files.

Gazelle provides a Go API to extend it with additional languages.
Access the [Extending Gazelle](https://github.com/bazel-contrib/bazel-gazelle/blob/master/extend.md).

However, this API requires some effort to learn and introduces a compile-time dependency for developers. The Aspect Extension Language (AXL), a Starlark dialect, allows you to write Starlark extensions that augment Gazelle.

## Define an extension

To create a Gazelle extension, you must first register the rule kinds that your extension will generate.


For example, to generate `BUILD` files that include:

```
load("//tools/oci:go_image.bzl", "go_image")
```


declare the rule kind:

```
aspect.register_rule_kind("go_image", {
    "From": "//tools/oci:go_image.bzl",
    "ResolveAttrs": ["binary"],
})
```

Next, define two functions for your extension:

- `prepare`: Specifies what information your extension needs from the source files, such as file extensions or regular expression queries.
- `declare`: Contains the logic to generate new build targets based on the collected information.

These functions are then passed to `aspect.register_configure_extension()`, which registers your extension with Gazelle. You can use named functions or anonymous lambdas as needed.

The following code continues the example:

```
def declare_targets(ctx):
    for file in ctx.sources:
        if len(file.query_results["has_main"]) > 0:
            ctx.targets.add(
                name = "image",
                kind = "go_image",
                attrs = {
                    "binary": path.base(ctx.rel),
                },
            )

aspect.register_configure_extension(
    id = "go_image",
    prepare = lambda cfg: aspect.PrepareResult(
        sources = [aspect.SourceExtensions(".go")],
        queries = {
            "has_main": aspect.RegexQuery(
                filter = "*.go",
                expression = """(?P<main>func\\s+main\\(.*?\\))""",
            ),
        },
    ),
    declare = declare_targets,
)
```
