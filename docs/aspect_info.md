## aspect info

Display runtime info about the bazel server

### Synopsis

Displays information about the state of the bazel process in the
form of several "key: value" pairs.  This includes the locations of
several output directories.  Because some of the
values are affected by the options passed to 'bazel build', the
info command accepts the same set of options.

Documentation: <https://bazel.build/docs/user-manual#info>

If arguments are specified, each should be one of the keys (e.g. "bazel-bin").
In this case only the value(s) for those keys will be printed.

If --show_make_env is specified, the output includes the set of key/value
pairs in the "Make" environment, accessible within BUILD files.

The full list of keys and the meaning of their values is documented in
the bazel User Manual, and can be programmatically obtained with
'aspect help info-keys'.

See also 'aspect version' for more detailed version information about the tool.

```
aspect info [keys] [flags]
```

### Options

```
  -h, --help            help for info
      --show_make_env   include the set of key/value pairs in the "Make" environment,
                        accessible within BUILD files
```

### Options inherited from parent commands

```
      --aspect:config string   config file (default is $HOME/.aspect/cli/config.yaml)
      --aspect:interactive     Interactive mode (e.g. prompts for user input)
```

### SEE ALSO

* [aspect](aspect.md)	 - Aspect.build bazel wrapper

