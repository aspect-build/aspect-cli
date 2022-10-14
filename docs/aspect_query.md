## aspect query

Query the dependency graph, ignoring configuration flags

### Synopsis

Executes a query language expression over a specified subgraph of the build dependency graph.

Note that this ignores the current configuration. Most users should use cquery instead,
unless you have a specific need to query the unconfigured graph.

```
aspect query [flags]
```

### Options

```
  -h, --help   help for query
```

### Options inherited from parent commands

```
      --aspect:config string   config file (default is $HOME/.aspect/cli/config.yaml)
      --aspect:interactive     Interactive mode (e.g. prompts for user input)
```

### SEE ALSO

* [aspect](aspect.md)	 - Aspect.build bazel wrapper

