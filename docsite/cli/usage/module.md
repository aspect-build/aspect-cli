

The `MODULE.aspect` file is a special Starlark file in the root of the repository, typically right next to a `MODULE.bazel` file.

## Dependencies

The `axl_dep` lets you declare a dependency, similar to the `bazel_dep` call in `MODULE.bazel`. For example, to use the `lint` and `format` commands provided by `rules_lint`:

```python
# Aspect Extension Language (AXL) dependencies; see https://github.com/aspect-extensions
axl_dep(
    name = "aspect_rules_lint",
    urls = ["https://github.com/aspect-build/rules_lint/archive/5837253fd2a1f86f952c44a50a1399813658d0a8.tar.gz"],
    integrity = "sha384-DIgszYfRk3aCPGPtgV5RSYCGwlgKWsFWU7z3F5Sm5xVXbS2f8WJk0Ri/WvSMjjFt",
    strip_prefix = "rules_lint-5837253fd2a1f86f952c44a50a1399813658d0a8",
    dev = True,
)
```

## Find shared extensions

Aspect maintains a dedicated GitHub organization at [Aspect Extensions](https://github.com/aspect-extensions), which hosts a collection of official and community-contributed extensions. The organization encourages developers to contribute their own extensions to this repository so that it's easier to enhance community collaboration and visibility.

Additionally, you can discover more extensions by exploring the [`aspect-extensions` topic on GitHub](https://github.com/topics/aspect-extensions).
This topic aggregates a wide range of extensions, making it easier to find tools tailored to your specific needs.
