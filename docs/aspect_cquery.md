## aspect cquery

Query the dependency graph, honoring configuration flags

### Synopsis

Executes a query language expression over a specified subgraph of the configured build dependency graph.

cquery should be preferred over query for typical usage, since it includes the analysis phase and
therefore provides results that match what the build command will do.

Note that cquery is especially powerful as the graph can be processed by a purpose-built program
written in Starlark. See <https://bazel.build/query/cquery#output-format-definition>.

Aspect CLI introduces the second form, where in place of an expression, you can give a preset query name.
Some preset queries also accept parameters, such as labels of targets, which can be provided as arguments.
If they are absent and the session is interactive, the user will be prompted to supply these.

Documentation: <https://bazel.build/query/cquery>


```
aspect cquery [expression |  <preset name> [arg ...]] [flags]
```

### Options

```
  -h, --help   help for cquery
```

### Options inherited from parent commands

```
      --aspect:config string   config file (default is $HOME/.aspect/cli/config.yaml)
      --aspect:interactive     Interactive mode (e.g. prompts for user input)
```

### SEE ALSO

* [aspect](aspect.md)	 - Aspect CLI

