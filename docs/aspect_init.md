## aspect init

Create a new Bazel workspace

### Synopsis

Creates a Bazel workspace.

It stamps out commonly needed files to get started more quickly with a brand-new project.

Folder may be a new directory to create, or "." to use the current working directory.
If omitted, the user is prompted to supply a value.

```
aspect init [folder] [flags]
```

### Options

```
  -h, --help   help for init
```

### Options inherited from parent commands

```
      --aspect:config string   User-specified Aspect CLI config file. /dev/null indicates that all further --aspect:config flags will be ignored.
      --aspect:interactive     Interactive mode (e.g. prompts for user input)
```

### SEE ALSO

* [aspect](aspect.md)	 - Aspect CLI

