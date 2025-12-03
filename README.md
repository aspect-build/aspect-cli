> [!WARNING]
> **Early Preview**<br>
> This is an early preview of Aspect CLI. It is under active development, and API changes are expected. We plan to stabilize the API and release a stable version by the end of 2025.

# Aspect CLI

Aspect CLI is a programmable task runner built on top of Bazel that "just fits" with your repository and developer workflows.

## Bazel's Shortcoming

Bazel is Google's powerful polyglot build system.

It's excellent at loading a dependency graph from package declarations (`query`/`cquery`), and analyzing an action graph from rule implementations (`aquery`).
It also has powerful and scalable execution for building action outputs and test results (`build`&`test`).

Bazel is extensible, but only for defining build rules that produce additional output files.
It falls short on customizing developer workflows.
In fact the Bazel commands not mentioned above are not-so-excellent attempts at adding a few developer workflows for use within Google.

The fact that many companies have scripted around Bazel, and the local development scripts drift from the CI testing scripts, show that something is missing: a "task runner" layer on top of `query` and `build` primitives.

## Introducing Aspect CLI

Aspect CLI lets you program custom commands using Aspect Extension Language (AXL), a Starlark dialect, for robust, maintainable workflows.
Say goodbye to brittle Bash wrappers and delete your `Makefile`.

- New engineers on the team don't struggle to setup their machine and run the series of commands to get a working build or reproduce what happened on CI.
- Product engineers finally regain control over their own productivity.
- Developer Infrastructure teams can stop chasing reports of misbehavior on CI and integrate great tooling into every developers routine.

Whether you're streamlining CI/CD, enforcing code standards, or integrating tools, Aspect CLI boosts productivity. It's fast, safe, and open source—try it today and transform how your team builds!

## Extensions

We publish extensions to https://github.com/aspect-extensions.

You can also search for the `aspect-extensions` topic:
http://github.com/topics/aspect-extensions

You can write your own extensions as well. See the documentation.

## Comparison to older versions

> [!NOTE]
> This is a Rust rewrite, superseding the legacy Go version that was published in version 2025.41 and earlier.
> The older implementation is now in maintenance mode at [aspect-build/aspect-cli-legacy](https://github.com/aspect-build/aspect-cli-legacy).

Versions before 2025.42 differed in some notable ways:

- Older versions shadowed the `bazel` command in our recommended installation, using homebrew or bazeliskrc to override `bazel`.
  Now `aspect` works alongside `bazel`.
- The plugin system used a gRPC client/server protocol. Now `aspect` uses a Starlark dialect called "Aspect Extension Language".
- Older versions included a fully pre-compiled Gazelle binary along with some Gazelle extensions, using the `configure` command. This has moved to a standalone repo: https://github.com/aspect-build/aspect-gazelle

## Licenses

Aspect CLI is licensed under [Apache 2](./LICENSE).

## For Enterprise

Built by [Aspect](http://aspect.build). Explore our products for advanced features.
