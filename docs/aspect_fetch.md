## aspect fetch

Fetch external repositories that are prerequisites to the targets

### Synopsis

Fetches all external dependencies for the targets given.

Note that Bazel uses the term "fetch" to mean both downloading remote files, and also running local
installation commands declared by rules for these external files.

Documentation: <https://bazel.build/run/build#fetching-external-dependencies>

If you observe fetching that should not be needed to build the
requested targets, this may indicate an "eager fetch" bug in some ruleset you rely on.
Read more: <https://blog.aspect.dev/avoid-eager-fetches>

```
aspect fetch <target patterns> [flags]
```

### Options

```
  -h, --help   help for fetch
```

### Options inherited from parent commands

```
      --aspect:config string   config file (default is $HOME/.aspect/cli/config.yaml)
      --aspect:interactive     Interactive mode (e.g. prompts for user input)
```

### SEE ALSO

* [aspect](aspect.md)	 - Aspect.build bazel wrapper

