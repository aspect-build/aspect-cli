## aspect modquery

Queries the Bzlmod external dependency graph.

### Synopsis

The command will display a dependency tree or parts of the dependency tree, structured to display different kinds of insights depending on the query type.
Calling the command with no argument will default to:
bazel modquery tree root

<query_type> [<args> ...] can be one of the following:

    - tree: Displays the full dependency tree. Use the --from option to specify which module(s) you want the tree to start from (defaults to root which displays the whole dependency tree).
    - deps <module(s)>: Displays the direct dependencies of the target module(s).
    - path <module(s)_to>: Displays the shortest path found in the dependency graph from (any of) the --from module(s) to (any of) <module(s)_to>.
    - all_paths <module(s)_to>: Display the dependency graph starting from (any of) the --from module(s) and containing any existing paths to (any of) the <module(s)_to>.
    - explain <module(s)>: Prints all the places where the module is (or was) requested as a direct dependency, along with the reason why the respective final version was selected. It will display a pruned version of the all_paths root <module(s)> command which only contains the direct deps of the root, the <module(s)> leaves, along with their dependants (can be modified with --depth).
    - show <module(s)>: Prints the rule that generated these modules? repos (i.e. http_archive()).

<module> arguments must be of type:

    - root: The current (root) module you are inside of.
    - <name>@<version>: A specific module version.
    - <name>@_: Specifies the empty version of a module (for non-registry overridden) modules).
    - <name>: Can be used as a placeholder for all the present versions of the module <name>.
    - <repo_name>: The repo_name of one of the root project?s direct dependencies, as it is defined in the MODULE.bazel file.

<modules> means:

    - <module>,<module>,... : A list of comma separated modules, where each <module> has the form of one of the above.

NOTE: This command is still very experimental and the precise semantics
will change in the near future.

```
aspect modquery [flags]
```

### Options

```
  -h, --help   help for modquery
```

### Options inherited from parent commands

```
      --aspect:config string   config file (default is $HOME/.aspect.yaml)
      --aspect:interactive     Interactive mode (e.g. prompts for user input)
```

### SEE ALSO

* [aspect](aspect.md)	 - Aspect.build bazel wrapper

