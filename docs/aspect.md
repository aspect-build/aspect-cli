---
sidebar_label: "aspect"
---
## aspect

Aspect CLI

### Synopsis

Aspect CLI is a better frontend for running bazel

### Options

```
      --aspect:config string   User-specified Aspect CLI config file. /dev/null indicates that all further --aspect:config flags will be ignored.
      --aspect:hints           Enable hints if configured (default true)
      --aspect:interactive     Interactive mode (e.g. prompts for user input)
  -h, --help                   help for aspect
```

### SEE ALSO

* [aspect analyze-profile](aspect_analyze-profile.md)	 - Analyze build profile data
* [aspect aquery](aspect_aquery.md)	 - Query the action graph
* [aspect build](aspect_build.md)	 - Build the specified targets
* [aspect canonicalize-flags](aspect_canonicalize-flags.md)	 - Present a list of bazel options in a canonical form
* [aspect clean](aspect_clean.md)	 - Remove the output tree
* [aspect config](aspect_config.md)	 - Displays details of configurations.
* [aspect configure](aspect_configure.md)	 - Auto-configure Bazel by updating BUILD files
* [aspect coverage](aspect_coverage.md)	 - Same as 'test', but also generates a code coverage report.
* [aspect cquery](aspect_cquery.md)	 - Query the dependency graph, honoring configuration flags
* [aspect docs](aspect_docs.md)	 - Open documentation in the browser
* [aspect fetch](aspect_fetch.md)	 - Fetch external repositories that are prerequisites to the targets
* [aspect info](aspect_info.md)	 - Display runtime info about the bazel server
* [aspect init](aspect_init.md)	 - Create a new Bazel workspace
* [aspect license](aspect_license.md)	 - Prints the license of this software.
* [aspect lint](aspect_lint.md)	 - Run configured linters over the dependency graph.
* [aspect mod](aspect_mod.md)	 - Tools to work with the bzlmod external dependency graph
* [aspect outputs](aspect_outputs.md)	 - Print paths to declared output files
* [aspect print](aspect_print.md)	 - Print syntax elements from BUILD files
* [aspect query](aspect_query.md)	 - Query the dependency graph, ignoring configuration flags
* [aspect run](aspect_run.md)	 - Build a single target and run it with the given arguments
* [aspect shutdown](aspect_shutdown.md)	 - Stop the bazel server
* [aspect test](aspect_test.md)	 - Build the specified targets and run all test targets among them
* [aspect version](aspect_version.md)	 - Print the versions of Aspect CLI and Bazel

