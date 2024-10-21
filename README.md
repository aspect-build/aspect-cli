---
title: Aspect CLI
sidebar_label: Overview
---

Aspect CLI (`aspect`) is wrapper for [Bazel], built on top of [Bazelisk], that adds additional features and extensibility to the popular polyglot build system from Google.

# Why Aspect CLI?

Every organization has a different engineering culture and developer stack.
Bazel was designed for Google's workflows, not yours.
Many companies have found they have to write a wrapper around Bazel.
This starts out as a small need to shim something in the developer workflow, and is often an
untested Bash script living in `/tools/bazel` which Bazelisk understands as a wrapper script.

Over time, the wrapper accumulates more code, and is a constant source of developer distress.

See more on our docsite: <https://docs.aspect.build/cli/>

# Licenses

# Aspect CLI OSS

Aspect CLI OSS, the open-source, open-core portion of the Aspect CLI, is found in this repository and is [Apache 2](./LICENSE) licensed.

# Aspect CLI

The standard Aspect CLI is built on top of the open-source, open-core portion found is this repository and is licensed under the [Aspect Community License](./ASPECT_COMMUNITY_LICENSE). The parts of the Aspect CLI that are not found in this repository are closed source.

We intend for Aspect CLI to remain free for individuals (only for personal use), Small Business (fewer than 50 employees), and non-profit or academic institutions. Please contact Aspect at https://aspect.build if you would like to use the Aspect CLI and fall outside of free use.

# Installation

## Aspect CLI OSS

### Bazelisk (MacOS / Linux / Windows)

Aspect CLI OSS can be installed in an existing Bazel workspace using [Bazelisk].

> [!NOTE]
> This approach doesn't provide the `aspect init` command, which has to run outside a Bazel workspace.

From the [OSS releases page](https://github.com/aspect-build/aspect-cli/releases),
copy the `.bazeliskrc` snippet into your `.bazeliskrc` file to install Aspect CLI OSS for all developers in the target repository.

The underlying version of Bazel can be configured in your `.bazelversion` file or the `BAZEL_VERSION` environment variable.

### Manual (MacOS / Linux / Windows)

On MacOS and Linux, you can download the Aspect CLI OSS `aspect` binary for your platform on our
[Releases](https://github.com/aspect-build/aspect-cli/releases) page and add it to your `PATH` manually.

Note, if you manually install for MacOS, you can bypass the "Unknown Developer" dialog by running
`xattr -c $(which aspect)` before launching `aspect`.

## Aspect CLI (standard)

### Homebrew (MacOS)

To install the Aspect CLI on MacOS, you can run

```sh
brew install aspect-build/aspect/aspect
```

This installs the `aspect` command and also links it to `bazel`, just like the [Bazelisk] installer does.

### Bazelisk (MacOS / Linux)

Aspect CLI can be installed in an existing Bazel workspace using [Bazelisk].

From the [releases page](https://docs.aspect.build/cli/releases/),
copy the `.bazeliskrc` snippet into your `.bazeliskrc` file to install Aspect CLI OSS for all developers in the target repository.

The underlying version of Bazel can be configured in your `.bazelversion` file or the `BAZEL_VERSION` environment variable.

> [!IMPORTANT]
> Windows releases for Aspect CLI standard will be available soon. In the meantime, please use Aspect CLI OSS releases on Windows.

# Usage

Just run `aspect help` to see the available commands.
Some are the standard ones you know from Bazel, and others are new, such as `print` and `docs`.

## Write a plugin

Aspect's plugin system allows you to fit Bazel into your team's development process,
with custom commands, behaviors, and integrations.

A plugin is any program (written in any language) that serves our gRPC protocol.
The easiest way to get started is to clone our
[starter template repo](https://github.com/aspect-build/aspect-cli-plugin-template).

See the [Plugin Documentation](./plugins.md) for more information on how to write a plugin.

# Need help or having issues?

If you think you've hit a bug please file a [Bug Report](https://github.com/aspect-build/aspect-cli/issues/new/choose).

You can also find us on [Bazel Slack](https://slack.bazel.build/) on the #aspect-build channel.

# For Enterprise

Aspect CLI is built by [Aspect](http://aspect.build).

See our website at <http://aspect.build> to learn more about our product offerings.

[Bazel]: http://bazel.build
[Bazelisk]: https://github.com/bazelbuild/bazelisk
