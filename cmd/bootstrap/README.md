# Aspect Bootstrap Utility

The `bootstrap` utility is a lightweight command-line application that is installed on a client
machine, allowing the client to easily initialze a Bazel workspace to use the `aspect-cli`.

## Use Case

The following represents the desired workflow for a client to start using the `aspect-cli`.

```sh
$ brew install aspect-cli
$ cd my_bazel_workspace
$ aspect init
# Start using aspect-cli functionality in the workspace
```

The install `brew install aspect-cli` invocation installs the `bootstrap` utility with the name
`apsect`.  The utility supports two commands, `version` and `init`. The `version` command prints the
version information for the `bootstrap` command-line application. The `init` command, when run from
an existing Bazel workspace, updates the `.bazelversion` with the appropriate directives to download
the `aspect-cli`.  Any commands not recognized by `bootstrap` are passed along to `aspect-cli`.

## Future Work

- Provide a wizard to help clients create a Bazel workspace.
