The `aspect` CLI is a drop-in replacement for the `bazel` CLI that comes with Bazel.

# Why Aspect CLI

Every organization has a different engineering culture and developer stack.
Bazel was designed for Google's workflows, not yours.
Many companies have found they have to write a wrapper around Bazel.
This starts out as a small need to shim something in the developer workflow, and is often an
untested Bash script living in `/tools/bazel` which Bazelisk understands as a wrapper script.

Over time, the wrapper accumulates more code, and is a constant source of developer distress.

See more on our product webpage: <https://aspect.build/cli>

# Installation

## MacOS

On MacOS, you can run

```sh
% brew install aspect-build/aspect/aspect
```

This installs the `aspect` command and also links it to `bazel`, just like the [bazelisk] installer does.

> We plan to have a standard "core" homebrew formula so this will just be `brew install aspect` in the future.

## Linux

On Linux, you can download the `aspect` binary on our [Releases](https://github.com/aspect-build/aspect-cli/releases) page and add it to your `PATH` manually. This also works on MacOS and Windows.

## Bazelisk

On any platform, so long as you already have [bazelisk] installed, you can have [bazelisk]
install the Aspect CLI just like it can install the standard Bazel CLI.
Add this to your `.bazeliskrc` in your project folder to install Aspect for all developers:

```
BAZELISK_BASE_URL=https://github.com/aspect-build/aspect-cli/releases/download
USE_BAZEL_VERSION=aspect/5.0.1
```

Note that in all cases, the `.bazelversion` file continues to indicate which version of the
Bazel tool is fetched and run beneath the wrapper.

## Manual

You can manually install a binary from our [GitHub Releases] page and put it in your PATH.

On MacOS you can bypass the "Unknown Developer" dialog by running

```shell
xattr -c $(which aspect)
```

before launching `aspect`.

# Usage

Just run `aspect help` to see the available commands.
Some are the standard ones you know from Bazel, and others are new, such as `print` and `docs`.

## Write a plugin

Aspect's plugin system allows you to fit Bazel into your team's development process,
with custom commands, behaviors, and integrations.

A plugin is any program (written in any language) that serves our gRPC protocol.
The easiest way to get started is to clone our
[starter template repo](https://github.com/aspect-build/aspect-cli-plugin-template).

See the [Plugin Documentation](./help/topics/plugins.md) for more information on how to write a plugin.

# Need help or having issues?

If you think you've hit a bug please file a [Bug Report](https://github.com/aspect-build/aspect-cli/issues/new/choose).

You can also find us on [Bazel Slack](https://slack.bazel.build/) on the #aspect-dev channel.

# For Enterprise

`aspect` is sponsored by Aspect Development, a Bazel consulting company.
If your organization needs more help to make your Bazel migration a success,
come find us at [aspect.dev](https://aspect.dev)

We also offer a Professional edition of the Aspect CLI.
This adds features that big codebases rely on, like BUILD file generation.
See our website at <http://aspect.build> to learn more about our offerings.

[bazel]: http://bazel.build
[github releases]: https://github.com/aspect-dev/aspect-cli/releases
[bazelisk]: https://github.com/bazelbuild/bazelisk
