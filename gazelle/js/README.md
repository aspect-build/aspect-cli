# JavaScript/TypeScript BUILD file generation

This package automates the creation and maintenance of BUILD files for JavaScript and TypeScript, using [rules_js](https://github.com/aspect-build/rules_js) and [rules_ts](https://github.com/aspect-build/rules_ts). It is a [Gazelle](https://github.com/bazelbuild/bazel-gazelle) `Language` implementation.

## Usage

This feature is included in the [Aspect CLI](https://github.com/aspect-build/aspect-cli), accessed with the [`configure` command](https://docs.aspect.build/cli/commands/aspect_configure).
It's also possible to build into your own Gazelle binary.

## Rules

Generated targets include:

-   `ts_project` targets for source, tests, and custom targets and their ts/js/npm dependencies
-   `npm_package` targets for projects within a pnpm workspace
-   `npm_link_all_packages` for linking npm packages

### Directives

See [Aspect CLI Directives](https://docs.aspect.build/cli/help/directives.md#JavaScript) for a list of supported directives.
