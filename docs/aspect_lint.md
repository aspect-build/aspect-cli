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
      --diff      Show unified diff instead of diff stats for fixes
      --fix       Auto-apply all fixes
      --fixes     Request fixes from linters (where supported) (default true)
  -h, --help      help for lint
      --machine   Request machine readable lint reports from linters (where supported)
      --quiet     Hide successful lint results
      --report    Request lint reports from linters (default true)
```

### Options inherited from parent commands

```
      --aspect:config string   User-specified Aspect CLI config file. /dev/null indicates that all further --aspect:config flags will be ignored.
      --aspect:hints           Enable hints if configured (default true)
      --aspect:interactive     Interactive mode (e.g. prompts for user input)
```

### SEE ALSO

* [aspect](aspect.md)	 - Aspect CLI

