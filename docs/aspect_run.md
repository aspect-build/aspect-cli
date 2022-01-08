## aspect run

Builds the specified target and runs it with the given arguments.

### Synopsis

'run' accepts any 'build' options, and will inherit any defaults
provided by .bazelrc.

If your script needs stdin or execution not constrained by the bazel lock,
use 'bazel run --script_path' to write a script and then execute it.


```
aspect run [flags]
```

### Options

```
  -h, --help   help for run
```

### Options inherited from parent commands

```
      --config string   config file (default is $HOME/.aspect.yaml)
      --interactive     Interactive mode (e.g. prompts for user input)
```

### SEE ALSO

* [aspect](aspect.md)	 - Aspect.build bazel wrapper

