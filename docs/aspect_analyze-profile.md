## aspect analyze-profile

Analyze build profile data

### Synopsis

Analyzes build profile data for the given profile data files.

Analyzes each specified profile data file and prints the results.  The
input files must have been produced by the 'bazel build
--profile=file' command.

By default, a summary of the analysis is printed.  For post-processing
with scripts, the --dump=raw option is recommended, causing this
command to dump profile data in easily-parsed format.

```
aspect analyze-profile [flags]
```

### Options

```
  -h, --help   help for analyze-profile
```

### Options inherited from parent commands

```
      --aspect:config string   config file (default is $HOME/.aspect/cli/config.yaml)
      --aspect:interactive     Interactive mode (e.g. prompts for user input)
```

### SEE ALSO

* [aspect](aspect.md)	 - Aspect.build bazel wrapper

