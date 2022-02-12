`aspect` is a drop-in replacement for the `bazel` CLI that comes with Bazel.

It provides a simpler, more intutive experience for developers new to Bazel,
while also adding the power tools that make advanced users more productive.

## Interactive

When running in an interactive terminal, `aspect` gives helpful prompts to
fix mistakes in how you run the tool, your Bazel configuration, or your code.

In this example, the Bazel configuration didn't allow a dependency because the
`visibility` attribute needed adjustment, which `aspect` offers to do for you:

<script id="asciicast-eL4HL3BZhobRD8U4UIRKzyb8R" src="https://asciinema.org/a/eL4HL3BZhobRD8U4UIRKzyb8R.js" async></script>

Some other examples of interactivity:
- offer to apply fixes to your source code that a compiler suggested
- suggest better usage, like using `bazel cquery` in place of `bazel query` or avoiding `bazel clean`
- list common subcommands or expressions

## Customize for your organization

Every organization has a different engineering culture and developer stack.
Bazel was designed for Google's workflows, not yours.
A plugin allows you to fit `aspect` into your teams development process.

![People working together on software](/people.png)

A vibrant ecosystem of plugins accelerates your Bazel migration.
You can also write your own plugins, which execute directly from your repository.

In this example, the error message from a badly written `genrule` was confusing, so a plugin
was written to provide more help:

<script id="asciicast-57gaElVKNlb0d8pyZ7JGBDZhL" src="https://asciinema.org/a/57gaElVKNlb0d8pyZ7JGBDZhL.js" async></script>

Some other uses of plugins include:
- stamp out new Bazel projects following your local conventions
- point error messages to your internal documentation
- add commands for deploying, linting, rebasing, or other common developer workflows
- understand where your developers get stuck and provide help

Plugins are any program, written in any language, that runs a gRPC server speaking our protocol. We use the [plugin system from HashiCorp](https://github.com/hashicorp/go-plugin). Read more in the [plugins documentation](/help/topics/plugins)

## Open source and no lock-in

You can rely on `aspect` to power your developer experience workflows.

It is a superset of what Bazel provides, so you can always go back to running `bazel` commands.

It is open-source, and free for use by individuals, non-profits, and small businesses.

## Expert help is a click away

`aspect` is sponsored by Aspect Development, a Bazel consulting company.
If your organization needs more help to make your Bazel migration a success,
come find us at [aspect.dev](https://aspect.dev)

The CLI makes it easy for developers to diagnose their broken build by asking
for help directly within their terminal session.

# Installation

## Using a package manager

> Coming soon

## Manual installation

Download a binary from our [GitHub Releases] page and put it in your PATH.

On MacOS you can bypass the "Unknown Developer" dialog by running

```shell
xattr -c $(which aspect)
```

before launching `aspect`.

# User Manuals

The commands are documented under [aspect](/aspect).

[Bazel]: http://bazel.build
[GitHub Releases]: https://github.com/aspect-dev/aspect-cli/releases
