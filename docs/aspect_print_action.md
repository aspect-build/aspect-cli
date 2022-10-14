## aspect print_action

Print the command line args for compiling a file

### Synopsis

Builds the specified targets and prints the extra actions for the given
targets. Right now, the targets have to be relative paths to source files,
and the --compile_one_dependency option has to be enabled.

This command accepts all valid options to 'build', and inherits defaults for
'build' from your .bazelrc.  If you don't use .bazelrc, don't forget to pass
all your 'build' options to 'print_action' too.

See 'bazel help target-syntax' for details and examples on how to
specify targets.

```
aspect print_action [flags]
```

### Options

```
  -h, --help   help for print_action
```

### Options inherited from parent commands

```
      --aspect:config string   config file (default is $HOME/.aspect/cli/config.yaml)
      --aspect:interactive     Interactive mode (e.g. prompts for user input)
```

### SEE ALSO

* [aspect](aspect.md)	 - Aspect.build bazel wrapper

