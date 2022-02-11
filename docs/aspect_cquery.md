## aspect cquery

Executes a cquery.

### Synopsis

Executes a query language expression over a specified subgraph of the build dependency graph using cquery.

```
aspect cquery [flags]
```

### Options

```
  -h, --help         help for cquery
  -k, --keep_going   Continue as much as possible after an error.  While the target that failed and those that depend on it cannot be analyzed, other prerequisites of these targets can be.
```

### Options inherited from parent commands

```
      --aspect:config string   config file (default is $HOME/.aspect.yaml)
      --aspect:interactive     Interactive mode (e.g. prompts for user input)
```

### SEE ALSO

* [aspect](aspect.md)	 - Aspect.build bazel wrapper

