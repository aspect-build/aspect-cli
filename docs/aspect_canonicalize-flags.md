---
sidebar_label: "canonicalize-flags"
---
## aspect canonicalize-flags

Present a list of bazel options in a canonical form

### Synopsis

This command canonicalizes a list of bazel options.
		
This is useful when you need a unique key to group Bazel invocations by their flags.

Read [the Bazel canonicalize-flags documentation](https://bazel.build/docs/user-manual#canonicalize-flags)

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
      --aspect:config string   User-specified Aspect CLI config file. /dev/null indicates that all further --aspect:config flags will be ignored.
      --aspect:hints           Enable hints if configured (default true)
      --aspect:interactive     Interactive mode (e.g. prompts for user input)
```

### SEE ALSO

* [aspect](aspect.md)	 - Aspect CLI

