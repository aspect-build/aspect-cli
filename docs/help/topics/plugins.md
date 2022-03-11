# Aspect CLI Plugins

The plugin system is based on the excellent system developed by HashiCorp for the `terraform` CLI.

## High-level design

A plugin is any program with a gRPC server that implements our plugin protocol.

We provide convenient support for writing plugins in Go, but this is not
required. You can write a plugin in any language.
Plugins are hosted and versioned independently from the aspect CLI.

The aspect CLI process spawns the plugin as a subprocess, then connects as a
gRPC client to it. The client and server run a negotiation protocol to determine
version compatibility and what capabilities the plugin provides.

You can read more about this archecture here:
<https://github.com/hashicorp/go-plugin/blob/master/README.md>

## Plugin configuration

In a `.aspectplugins` file at the repository root, list the plugins you'd like to install.

This is a YAML file. The shortest example provides a name and a local path to the plugin binary:

```yaml
- name: some-plugin
  from: ./path/to/plugin_binary
```

The `from` line may start with `//` in which case it is interpreted as a [Bazel Label] in the
current workspace.
That label must be a `*_binary` rule which builds a plugin binary. When the CLI loads this
plugin, it first builds it from source.
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

## Capabilities

More documentation on the plugin API coming soon!
