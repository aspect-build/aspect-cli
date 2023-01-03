---
sidebar_label: "run"
---
## aspect run

Build a single target and run it with the given arguments

### Synopsis

Equivalent to `aspect build <target>` followed by spawning the resulting executable.

Documentation: <https://bazel.build/docs/user-manual#running-executables>

Two environment variables will be present that the program may reference:
- `BUILD_WORKSPACE_DIRECTORY`: the root of the workspace where the build was run.
- `BUILD_WORKING_DIRECTORY`: the current working directory where Bazel was run from.

Note that the `<target>`may have an `args` and `env`attribute. The `run` command honors these and
sets the arguments and environment of the spawned executable, unlike if the binary is executed as
an action during a build step, or is run directly outside of Bazel.
See <https://bazel.build/reference/be/common-definitions#common-attributes-binaries>.

`run` accepts any `build` options, and will inherit any defaults provided by `.bazelrc.`

If your script needs stdin or execution not constrained by the bazel lock,
use `bazel run --script_path` to write a script and then execute it.

By default, the program is run with a working directory inside $(aspect info execution_root).
Some programs expect to be run under a certain working directory, such as the workspace root.
Use the [--run_under](https://bazel.build/docs/user-manual#run_under) flag with a cd command, like
`aspect run --run_under="cd $PWD &&" //my:program` to use the current directory.
Another common approach if the program's code is in your repo (first-party) is to check for the
presence of `BUILD_WORKSPACE_DIRECTORY` in the environment, then change the working
directory of the process. You'd typically do this at the very beginning of the program execution.


```
aspect run [--run_under=command-prefix] <target> -- [args for program ...]
```

### Options

```
  -h, --help   help for run
```

### Options inherited from parent commands

```
      --aspect:config string   User-specified Aspect CLI config file. /dev/null indicates that all further --aspect:config flags will be ignored.
      --aspect:interactive     Interactive mode (e.g. prompts for user input)
```

### SEE ALSO

* [aspect](aspect.md)	 - Aspect CLI

