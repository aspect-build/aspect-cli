---
sidebar_label: "print"
---
## aspect print

Print syntax elements from BUILD files

### Synopsis

Syntactic printer for BUILD file content.

Unlike commands like `query --output=build`, print never runs the Bazel loading and analysis phases.
This means that print commands will return quickly, never needing to fetch external repositories,
run repository rules, or perform other expensive analysis operations.
It also tolerates incorrect BUILD files, such as those with invalid syntax or misspelled attributes.

On the other hand, since it does not evaluate macros, it only shows syntax that appears directly in
the BUILD file. If you want to evaluate macros, use `query --output=build` instead.

[targets] are similar to the label syntax for other commands. Differences include:
- you can refer to rules of a certain kind using `%`, e.g. `//pkg:%go_library`
- you can refer to a package using the pseudo-target `__pkg__`, e.g. `//pkg:__pkg__` 

The --output flag may accept multiple comma-separated values and may be repeated.
Values may be one of:

- rule: the entire rule definition (default)
- kind: displays the name of the function
- label: the fully qualified label
- startline: the line number on which the rule begins in the BUILD file
- endline: the line number on which the rule ends in the BUILD file
- path: the absolute path to the BUILD file that contains the rules

print uses the same library as 'buildozer' so this documentation is relevant as well:
https://github.com/bazelbuild/buildtools/blob/master/buildozer/README.md#print-commands

```
aspect print [--output=...] <targets> [flags]
```

### Examples

```
# Print the entire definition (including comments) of the //base:heapcheck rule:
aspect print //base:heapcheck

# Print the kind of a target
aspect print --output=kind base  # output: cc_library

# Print the name of all go_library targets in //base
aspect print --output=name base:%go_library

# Get the default visibility of the //base package
aspect print --output=default_visibility base:%package

# Print labels of go_library targets under //cli that have a deps attribute
aspect print --output=label,deps //cli/...:%go_library 2>/dev/null | cut -d' ' -f1

# Print the list of labels in //base that explicitly set the testonly attribute:
aspect print --output=label --output=testonly 'base:*' 2>/dev/null
```

### Options

```
  -h, --help             help for print
      --output strings   Syntax elements to print (default [rule])
```

### Options inherited from parent commands

```
      --aspect:config string   User-specified Aspect CLI config file. /dev/null indicates that all further --aspect:config flags will be ignored.
      --aspect:interactive     Interactive mode (e.g. prompts for user input)
```

### SEE ALSO

* [aspect](aspect.md)	 - Aspect CLI

