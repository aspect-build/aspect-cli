

The `aspect` command-line interface supercharges your terminal experience. It enhances Bazel's build and query functionalities, enabling seamless integration with the Developer Workflows your team relies on. Unlike ad-hoc shell scripts, it provides a robust and intuitive abstraction that aligns perfectly with Bazel's design principles.

You can find the source code, report issues, and download the latest releases at the [Aspect CLI GitHub repository](https://github.com/aspect-build/aspect-cli).

## Aspect extension language

The Aspect Extension Language (AXL) is a specialized dialect of [Starlark](https://starlark-lang.org/) designed to configure and extend the Aspect CLI.

Just as Bazel uses `.bzl` files to define Starlark-based configurations with [Bazel's built-in library](https://bazel.build/rules/lib/builtins), the Aspect CLI employs `.axl` files to create and manage its extensions. This parallel ensures a seamless transition for developers familiar with Bazel's ecosystem while leveraging the power of Aspect's specialized tooling.

AXL takes inspiration from the Buck Extension Language (BXL) and tools like [Tilt](https://docs.tilt.dev/api.html), both of which leverage Starlark as their configuration language. By adopting this approach, AXL provides a consistent, intuitive experience for developers familiar with Starlark-based ecosystems, while introducing enhancements tailored to Aspect's unique capabilities.

## Task extensions

This AXL interface enables you to define custom commands for the `aspect` CLI, extending its capabilities to suit your specific workflows. The same extensible library implements built-in commands, ensuring consistency and ease of use.

## Gazelle extensions

The AXL interface enhances Bazel's `BUILD` file generator by allowing you to incorporate custom business logic written in Starlark.

For an in-depth guide on creating these extensions, check out the [Aspect 301 training course](/training/aspect-301).

## API reference

Explore the [full API documentation](/axl) to dive deeper into the available methods, properties, and examples for leveraging AXL in your projects.
