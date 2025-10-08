---
sidebar_label: "configure"
---
## aspect configure

Auto-configure Bazel by updating BUILD files

### Synopsis

configure generates and updates BUILD files from source code.

It is named after the "make configure" workflow which is typical in C++ projects, using
[autoconf](https://www.gnu.org/software/autoconf/).

configure is non-destructive: hand-edits to BUILD files are generally preserved.
You can use a `# keep` directive to force the tool to leave existing BUILD contents alone.
Run 'aspect help directives' for more documentation on directives.

So far these languages are supported:
- Go and Protocol Buffers, thanks to code from [gazelle]
- Python, thanks to code from [rules_python]
- JavaScript (including TypeScript)
- Kotlin (experimental, see https://github.com/aspect-build/aspect-cli/issues/474)
- Starlark, thanks to code from [bazel-skylib]

configure is based on [gazelle]. We are very grateful to the authors of that software.
The advantage of configure in Aspect CLI is that you don't need to compile the tooling before running it.

[gazelle]: https://github.com/bazelbuild/bazel-gazelle
[rules_python]: https://github.com/bazelbuild/rules_python/tree/main/gazelle
[bazel-skylib]: https://github.com/bazelbuild/bazel-skylib/tree/main/gazelle

To change the behavior of configure, you add "directives" to your BUILD files, which are comments
in a special syntax.
Run 'aspect help directives' or see https://github.com/aspect-build/aspect-cli/blob/main/docs/aspect_configure.md for more info.


```
aspect configure [flags]
```

### Options

```
      --exclude strings   Files to exclude from BUILD generation
  -h, --help              help for configure
      --mode string       Method for emitting merged BUILD files.
                          	fix: write generated and merged files to disk
                          	print: print files to stdout
                          	diff: print a unified diff (default "fix")
      --watch             Use the EXPERIMENTAL watch mode to watch for changes in the workspace and automatically 'configure' when files change
      --watchman          Use the EXPERIMENTAL watchman daemon to watch for changes across 'configure' invocations
```

### Options inherited from parent commands

```
      --aspect:config string   User-specified Aspect CLI config file. /dev/null indicates that all further --aspect:config flags will be ignored.
      --aspect:hints           Enable hints if configured (default true)
      --aspect:interactive     Interactive mode (e.g. prompts for user input)
```

### SEE ALSO

* [aspect](aspect.md)	 - Aspect CLI

