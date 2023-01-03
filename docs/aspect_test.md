---
sidebar_label: "test"
---
## aspect test

Build the specified targets and run all test targets among them

### Synopsis

Runs test targets and reports the test results.

Documentation: <https://bazel.build/docs/user-manual#running-tests>

First, the targets are built. See 'aspect help build' for more about the phases of a build.
By default, any targets that match the pattern(s) are built, even if they are not needed as inputs
to any test target. Use `--build_tests_only` to avoid building these targets.

Targets may be filtered from the pattern. See <https://bazel.build/docs/user-manual#test-selection>:
- by size, using `--test_size_filters` often used to select only "unit tests"
- by timeout, using `--test_timeout_filters` often used to select only fast tests,
- by tag, using `--test_tag_filters`
- by language, using `--test_lang_filters` though it only understands those built-in to Bazel.
  Follow https://github.com/bazelbuild/bazel/issues/12618

The tests are run following a well-specified contract between Bazel and the test runner process, see
<https://bazel.build/reference/test-encyclopedia>

This command accepts all valid options to 'build', and inherits
defaults for 'build' from your .bazelrc.  If you don't use .bazelrc,
don't forget to pass all your 'build' options to 'test' too.

See 'aspect help target-syntax' for details and examples on how to specify targets.


```
aspect test [--build_tests_only] <target pattern> [<target pattern> ...] [flags]
```

### Options

```
  -h, --help   help for test
```

### Options inherited from parent commands

```
      --aspect:config string   User-specified Aspect CLI config file. /dev/null indicates that all further --aspect:config flags will be ignored.
      --aspect:interactive     Interactive mode (e.g. prompts for user input)
```

### SEE ALSO

* [aspect](aspect.md)	 - Aspect CLI

