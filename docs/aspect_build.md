---
sidebar_label: "build"
---
## aspect build

Build the specified targets

### Synopsis

Performs a build on the specified targets, producing their default outputs.

Read [the Bazel build documentation](https://bazel.build/run/build#bazel-build)

Run 'aspect help target-syntax' for details and examples on how to specify targets to build.

Commonly used flags
-------------------

Bazel will first fetch any missing or out-of-date external dependencies.
You can run build with `--fetch=false` to inhibit this.
See 'aspect help fetch' for more information.

Since Bazel has no analyze command, you can use `build --nobuild` to only load and analyze
BUILD files without spawning any build actions. See https://github.com/bazelbuild/bazel/issues/15318

The build will halt as soon as the first error is encountered. Use `--keep_going (-k)` to
continue building.

Note that the rule implementation(s) may only run a subset of their actions to produce the default
outputs of the requested targets.
To create non-default outputs, consider using the `--output_groups` flag.

The target pattern may be further filtered using the flag
[--build_tag_filters](https://bazel.build/reference/command-line-reference#flag--build_tag_filters)


```
aspect build <target patterns> [flags]
```

### Options

```
  -h, --help   help for build
```

### Options inherited from parent commands

```
      --aspect:config string   User-specified Aspect CLI config file. /dev/null indicates that all further --aspect:config flags will be ignored.
      --aspect:interactive     Interactive mode (e.g. prompts for user input)
```

### SEE ALSO

* [aspect](aspect.md)	 - Aspect CLI

