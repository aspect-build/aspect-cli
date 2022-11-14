## aspect docs

Open documentation in the browser

### Synopsis

Given a selected topic, open the relevant API docs in a browser window.

The mechanism of choosing the browser to open is documented at https://github.com/pkg/browser
By default, opens bazel.build/docs

```
aspect docs [topic] [flags]
```

### Examples

```
# Open the Bazel glossary of terms
% aspect docs glossary

# Open the docs for the aspect-build/rules_js ruleset
% aspect docs rules_js
```

### Options

```
  -h, --help   help for docs
```

### Options inherited from parent commands

```
      --aspect:config string   User-specified Aspect CLI config file. /dev/null indicates that all further --aspect:config flags will be ignored.
      --aspect:interactive     Interactive mode (e.g. prompts for user input)
```

### SEE ALSO

* [aspect](aspect.md)	 - Aspect CLI

