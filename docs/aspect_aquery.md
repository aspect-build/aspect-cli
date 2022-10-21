## aspect aquery

Query the action graph

### Synopsis

Executes a query language expression over a specified subgraph of the action graph.

Documentation: <https://bazel.build/query/aquery>

Aspect CLI introduces the second form, where in place of an expression, you can give a preset query name.
Some preset queries also accept parameters, such as labels of targets, which can be provided as arguments.
If they are absent and the session is interactive, the user will be prompted to supply these.

```
aspect aquery [expression |  <preset name> [arg ...]] [flags]
```

### Examples

```
# Get the action graph generated while building //src/target_a
$ aspect aquery '//src/target_a'

# Get the action graph generated while building all dependencies of //src/target_a
$ aspect aquery 'deps(//src/target_a)'

# Get the action graph generated while building all dependencies of //src/target_a
# whose inputs filenames match the regex ".*cpp".
$ aspect aquery 'inputs(".*cpp", deps(//src/target_a))'
```

### Options

```
  -h, --help   help for aquery
```

### Options inherited from parent commands

```
      --aspect:config string   config file (default is $HOME/.aspect/cli/config.yaml)
      --aspect:interactive     Interactive mode (e.g. prompts for user input)
```

### SEE ALSO

* [aspect](aspect.md)	 - Aspect.build bazel wrapper

