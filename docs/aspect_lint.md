---
sidebar_label: "lint"
---
## aspect lint

Run configured linters over the dependency graph.

### Synopsis

Run linters and collect the reports they produce.

To setup linters, see the documentation on https://github.com/aspect-build/rules_lint

In addition to flags listed below, flags accepted by the 'bazel build' command are also accepted.


```
aspect lint <target patterns> [flags]
```

### Options

```
      --diff            Output patch fixes for lint errors
      --fix             Apply patch fixes for lint errors
  -h, --help            help for lint
      --output string   Format for output of lint reports, either 'text' or 'sarif' (default "text")
      --report          Output lint reports (default true)
```

### Options inherited from parent commands

```
      --aspect:config string   User-specified Aspect CLI config file. /dev/null indicates that all further --aspect:config flags will be ignored.
      --aspect:interactive     Interactive mode (e.g. prompts for user input)
```

### SEE ALSO

* [aspect](aspect.md)	 - Aspect CLI

