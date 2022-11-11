## aspect canonicalize-flags

Present a list of bazel options in a canonical form

### Synopsis

This command canonicalizes a list of bazel options.
		
This is useful when you need a unique key to group Bazel invocations by their flags.

Documentation: <https://bazel.build/docs/user-manual#canonicalize-flags>

```
aspect canonicalize-flags -- <bazel flags>
```

### Examples

```
% aspect canonicalize-flags -- -k -c opt
--keep_going=1
--compilation_mode=opt
```

### Options

```
  -h, --help   help for canonicalize-flags
```

### Options inherited from parent commands

```
      --aspect:config string   config file (default is $HOME/.aspect/cli/config.yaml)
      --aspect:interactive     Interactive mode (e.g. prompts for user input)
```

### SEE ALSO

* [aspect](aspect.md)	 - Aspect CLI

