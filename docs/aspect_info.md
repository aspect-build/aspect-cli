## aspect info

Displays runtime info about the bazel server.

### Synopsis

Displays information about the state of the bazel process in the
form of several "key: value" pairs.  This includes the locations of
several output directories.  Because some of the
values are affected by the options passed to 'bazel build', the
info command accepts the same set of options.

A single non-option argument may be specified (e.g. "bazel-bin"), in
which case only the value for that key will be printed.

If --show_make_env is specified, the output includes the set of key/value
pairs in the "Make" environment, accessible within BUILD files.

The full list of keys and the meaning of their values is documented in
the bazel User Manual, and can be programmatically obtained with
'bazel help info-keys'.

See also 'bazel version' for more detailed bazel version
information.

```
aspect info [flags]
```

### Options

```
  -h, --help            help for info
      --show_make_env   include the set of key/value pairs in the "Make" environment,
                        accessible within BUILD files
```

### Options inherited from parent commands

```
      --aspect:config string   config file (default is $HOME/.aspect.yaml)
      --aspect:interactive     Interactive mode (e.g. prompts for user input)
```

### SEE ALSO

* [aspect](aspect.md)	 - Aspect.build bazel wrapper

