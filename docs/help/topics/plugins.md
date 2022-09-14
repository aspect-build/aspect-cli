# Aspect CLI Plugins

Plugins allow you to customize Bazel's behavior, and they're easy to write!
A plugin can subscribe to the Build Event Protocol (BEP), to react in real-time during the build.
Plugins can contribute custom commands like `lint` so developers can live in a single tool.

## High-level design

A plugin is any program with a gRPC server that implements our plugin protocol.

We provide convenient support for writing plugins in Go, but this is not required.
You can write a plugin in any language.
Plugins are hosted and versioned independently from the aspect CLI.

The aspect CLI process spawns the plugin as a subprocess, then connects as a
gRPC client to it. The client and server run a negotiation protocol to determine
version compatibility and what capabilities the plugin provides.

The plugin system is based on the excellent system developed by HashiCorp for the `terraform` CLI.
You can read more about this archecture here:
<https://github.com/hashicorp/go-plugin/blob/master/README.md>

## Quickstart

Use the https://github.com/aspect-build/aspect-cli-plugin-template repo to create a starter repo.

Follow instructions on the README to customize the plugin for your org.

## Plugin configuration

In a `.aspectplugins` file at the repository root, list the plugins you'd like to install.

This is a YAML file. The shortest example provides a name and a local path to the plugin binary:

```yaml
- name: cool-plugin
  from: my-aspect-plugin
```

The `from` line points to the plugin binary and can take one of these forms:

1. A program on your system `PATH`.
2. A filesystem path, either relative to the `.aspectplugins` file or absolute.
3. A string starting with `//` in which case it is interpreted as a [Bazel Label] in the
   current workspace.
4. An http/https URL where the plugin can be downloaded from.
   To get a binary for the right platform, we append one of these
   platform suffixes before fetching:
   `-darwin_amd64`, `-darwin_arm64`, `-linux_amd64`, `-linux_arm64`, `-windows_amd64.exe`

When the `from` line is a label, it must be a `*_binary` rule which builds a plugin binary.
When the CLI loads this plugin, it first builds it from source.
This is useful as a local development round-trip while authoring a plugin. However, it is not a
great way to deploy a plugin to users, as it causes them to perform an extra build every time
they run `aspect`, whether they intend to use the plugin or not.

In the future, we'll add more ways to specify a plugin, such as with a remote URL.
This would use semantic versioning ranges to constrain the versions which can be used.
When aspect runs, it can then prompt you to re-lock the dependencies to exact versions if they
have changed, and can verify the integrity of the plugin contents against what was first installed.

> The locking semantics follow the [Trust on first use] approach.

[trust on first use]: https://en.wikipedia.org/wiki/Trust_on_first_use
[bazel label]: https://bazel.build/concepts/labels
