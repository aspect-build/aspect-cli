## aspect dump

Dump the internal state of the bazel server process

### Synopsis

Dumps the internal state of the bazel server process.

This command is provided as an aid to debugging, not as a stable interface, so
users should not try to parse the output; instead, use 'query' or 'info' for
this purpose.

```
aspect dump [flags]
```

### Options

```
  -h, --help   help for dump
```

### Options inherited from parent commands

```
      --aspect:config string   config file (default is $HOME/.aspect/cli/config.yaml)
      --aspect:interactive     Interactive mode (e.g. prompts for user input)
```

### SEE ALSO

* [aspect](aspect.md)	 - Aspect.build bazel wrapper

