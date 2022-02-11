## aspect build

Builds the specified targets, using the options.

### Synopsis

Invokes bazel build on the specified targets. See 'bazel help target-syntax' for details and examples on how to specify targets to build.

```
aspect build [flags]
```

### Options

```
  -h, --help         help for build
  -k, --keep_going   Continue as much as possible after an error.  While the target that failed and those that depend on it cannot be analyzed, other prerequisites of these targets can be.
```

### Options inherited from parent commands

```
      --aspect:config string   config file (default is $HOME/.aspect.yaml)
      --aspect:interactive     Interactive mode (e.g. prompts for user input)
```

### SEE ALSO

* [aspect](aspect.md)	 - Aspect.build bazel wrapper

