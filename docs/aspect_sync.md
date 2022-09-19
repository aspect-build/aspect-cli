## aspect sync

Syncs all repositories specified in the workspace file.

### Synopsis

Ensures that all Starlark repository rules of the top-level WORKSPACE
file are called.

NOTE: This command is still very experimental and the precise semantics
will change in the near future.

```
aspect sync [flags]
```

### Options

```
  -h, --help   help for sync
```

### Options inherited from parent commands

```
      --aspect:config string   config file (default is $HOME/.aspect/cli/config.yaml)
      --aspect:interactive     Interactive mode (e.g. prompts for user input)
```

### SEE ALSO

* [aspect](aspect.md)	 - Aspect.build bazel wrapper

