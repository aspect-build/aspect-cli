## aspect coverage

Same as 'test', but also generates a code coverage report.

### Synopsis

To produce a coverage report, use bazel coverage --combined_report=lcov [target].
This runs the tests for the target, generating coverage reports in the lcov format for each file.

Once finished, bazel runs an action that collects all the produced coverage files,
and merges them into one, which is then finally created under
$(bazel info output_path)/_coverage/_coverage_report.dat.

Coverage reports are also produced if tests fail, though note that this does not extend to the
failed tests - only passing tests are reported.

More documentation on gathering code coverage data with Bazel:
<https://bazel.build/configure/coverage>

See 'aspect help target-syntax' for details and examples on how to specify targets.


```
aspect coverage --combined_report=<value> <target pattern> [<target pattern> ...] [flags]
```

### Options

```
  -h, --help   help for coverage
```

### Options inherited from parent commands

```
      --aspect:config string   config file (default is $HOME/.aspect/cli/config.yaml)
      --aspect:interactive     Interactive mode (e.g. prompts for user input)
```

### SEE ALSO

* [aspect](aspect.md)	 - Aspect CLI

