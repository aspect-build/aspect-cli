---
sidebar_label: "analyze-profile"
---
## aspect analyze-profile

Analyze build profile data

### Synopsis

Analyzes build profile data for the given profile data file(s).

Analyzes each specified profile data file and prints the results.
The profile is commonly written to `$(bazel info output_base)/command.profile.gz`
after a build command completes.
You can use the `--profile=<file>` flag to supply an alternative path where the profile is written.

This command just dumps profile data to stdout. To inspect a profile you may want to use a GUI
instead, such as the `chrome//:tracing` interface built into Chromium / Google Chrome, or
<https://ui.perfetto.dev/>.

By default, a summary of the analysis is printed.  For post-processing
with scripts, the `--dump=raw` option is recommended, causing this
command to dump profile data in easily-parsed format.

```
aspect analyze-profile <command.profile.gz> [flags]
```

### Options

```
  -h, --help   help for analyze-profile
```

### Options inherited from parent commands

```
      --aspect:config string   User-specified Aspect CLI config file. /dev/null indicates that all further --aspect:config flags will be ignored.
      --aspect:interactive     Interactive mode (e.g. prompts for user input)
```

### SEE ALSO

* [aspect](aspect.md)	 - Aspect CLI

