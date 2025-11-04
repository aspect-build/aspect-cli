

Once you [install](../install) the Aspect CLI, you can run `aspect help` to view the list of available commands.

For example:

```shell
% aspect help
Usage: aspect <COMMAND>

Commands:
  multi_run  multi_run task defined in .aspect/multi_run.axl
  run        run task defined in .aspect/run.axl
  version    
  help       Print this message or the help of the given subcommand(s)
```

Some commands are built-in, while others are downloaded on-demand. Each command shows the file where it is defined.


## Extension Discovery

The Aspect CLI locates extensions in specific configuration locations:

- `.aspect` directories
- `*.axl` files

The Aspect CLI searches for `.aspect` directories starting from the current working directory and traversing upward to the topmost directory of the current Bazel project.
This allows teams to isolate commands to specific subdirectories when project-wide commands don't make sense.
Each `.aspect` directory can contain `.axl` files which contain extension code.

Additionally, the repository may contain a MODULE.aspect file. This defines external locations where extensions may be fetched.

For example:

```plaintext
.
├── .aspect/
|   ├── build.axl
│   └── mycmd.axl # defines a 'mycmd' command
├── app1/
│   └── .aspect/
│       └── mycmd.axl # overrides the 'mycmd' command when the working directory is beneath 'app1'
├── MODULE.aspect
└── MODULE.bazel
```

When you `cd app1` and then run `aspect mycmd`, the Aspect CLI first loads commands from `./app1/.aspect`, then traverses up the tree to `./.aspect/` and lists all available commands it finds. As a result, the 'build' command comes from the repository root, while the 'mycmd' command comes from the app1 folder.

## Write your first extension

Write extensions in the Aspect Extension Language (AXL) to power commands and enable custom workflows tailored to your project's needs. 

Follow these steps to create a custom `mycmd` command that wraps Bazel's build capabilities:

1. Create a directory at the root of the project called `.aspect`:

```shell
mkdir .aspect
```
2. Create a file within the new `.aspect` directory called `mycmd.axl`:

```shell
touch .aspect/mycmd.axl
```
3. Add the following Starlark contents into the new `mycmd.axl` file:

```python
def impl(ctx: task_context) -> int:
    build = ctx.bazel.build(events = True, *ctx.args.targets)

    for event in build.events():
        if event.kind == "named_set_of_files":
            for file in event.payload.files:
                ctx.std.io.stdout.write("Built {}\n".format(file.file))

    build.wait()
    return 0

mycmd = task(
    implementation = impl,
    args = {
        "targets": args.positional(),
    }
)
```
4. Validate the new command:

```shell
% aspect help
Usage: aspect <COMMAND>

Commands:
  mycmd      mycmd task defined in .aspect/multi_run.axl
  ...
  help       Print this message or the help of the given subcommand(s)
```

5. Run the following command in your terminal to test the new AXL command:

```shell
aspect mycmd //...
```

This command builds all targets and then lists the output files reported by Bazel.
