> [!TIP]
> Meet the Aspect team at [BazelCon 2025, November 9-11](https://events.linuxfoundation.org/bazelcon/)-—we'll demo the CLI! Training & Community Day is November 9.

# Aspect CLI

This repository contains the Aspect CLI, a programmable task engine built on top of Bazel. Written in Rust for speed and reliability, it empowers developers to customize and extend their build workflows seamlessly.

> [!WARNING]
> **Early Preview**<br>
> This is an early preview of the Aspect CLI rewritten in Rust. It is under active development, and API changes are expected. We plan to stabilize the API and release a stable version in November 2025.

## Why Aspect CLI?

Bazel is Google's powerful polyglot build system, but while it's extensible for defining build rules, it falls short on customizing developer workflows—often forcing teams into brittle Bash wrappers for unique needs. The Aspect CLI unlocks true extensibility for all of your developer workflows. Say goodbye to brittle Bash wrappers--Aspect CLI lets you program custom commands using AXL (Aspect Extension Language), powered by Starlark, for robust, maintainable workflows.

Whether you're streamlining CI/CD, enforcing code standards, or integrating tools, Aspect CLI boosts productivity. It's fast, safe, and open source—try it today and transform how your team builds!

> [!NOTE]
> This is the improved Rust-based implementation, superseding the legacy Go version now in maintenance mode at [aspect-build/aspect-cli-legacy](https://github.com/aspect-build/aspect-cli-legacy).

## Licenses

Aspect CLI is licensed under [Apache 2](./LICENSE).

## For Enterprise

Built by [Aspect](http://aspect.build). Explore our products for advanced features.
