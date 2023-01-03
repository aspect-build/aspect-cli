---
sidebar_label: "query"
---
## aspect query

Query the dependency graph, ignoring configuration flags

### Synopsis

Executes a query language expression over a specified subgraph of the unconfigured build dependency graph.

Note that this ignores the current configuration. Most users should use cquery instead,
unless you have a specific need to query the unconfigured graph.

Documentation: <https://bazel.build/query/quickstart>

```
aspect query [expression |  <preset name> [arg ...]] [flags]
```

### Options

```
  -h, --help   help for query
```

### Options inherited from parent commands

```
      --aspect:config string   User-specified Aspect CLI config file. /dev/null indicates that all further --aspect:config flags will be ignored.
      --aspect:interactive     Interactive mode (e.g. prompts for user input)
```

### SEE ALSO

* [aspect](aspect.md)	 - Aspect CLI

