# AXL Design

AXL is the configuration and task language for Aspect CLI. It uses [Starlark](https://github.com/bazelbuild/starlark) — a deterministic, hermetic dialect of Python — as its foundation, extended with domain-specific APIs for Bazel integration, process execution, HTTP, filesystem access, and more.

## Core Principle: Separation of Evaluation and Execution

AXL enforces a strict boundary between two fundamentally different modes of operation: **evaluation** and **execution**. This separation is the most important architectural decision in AXL and informs every API design choice.

### Evaluation

Evaluation is the process of loading and interpreting `.axl` files to produce Starlark heaps containing task definitions, trait types, and configuration functions. Evaluation has **no side effects**. It cannot make network calls, read from the filesystem, access environment variables, or perform any operation whose result could vary between runs given the same inputs.

The only things available during evaluation are:

- Standard Starlark builtins (`len`, `range`, `str`, `list`, `dict`, `True`, `False`, `None`, etc.)
- `json.encode()` / `json.decode()` (provided by starlark-rust)
- `load()` for importing other `.axl` files
- AXL definition functions: `task()`, `trait()`, `attr()`, `record()`, `config()`
- The `args` module for declaring task arguments

Everything reachable during evaluation must be **pure and deterministic**: given identical inputs, the output must be identical every time. This property is critical because:

1. **Cacheability.** Evaluated heaps can be cached by a daemon process across CLI invocations. If evaluation had side effects, cached results could silently diverge from fresh evaluation.
2. **Predictability.** Task definitions, trait schemas, and configuration shapes are stable artifacts. Users and tooling can reason about them without running anything.

When new APIs are introduced to the evaluation-time global scope (e.g., `yaml.decode`), they must satisfy this purity constraint. Functions like `uuid.v4()` or `time.now()` are inherently non-deterministic and are therefore **forbidden during evaluation**. Pure utility modules (hashing, math, regex) will be available via `load('@std//hash.axl', ...)` rather than as globals, to keep the default namespace minimal.

### Config Execution

After evaluation produces task definitions and trait types, the runtime may choose to **execute** the `config()` function. This is an explicit execution step — not evaluation — even though it runs before any user-invoked task. The config function receives a `ConfigContext` with access to:

- `ctx.http()` — HTTP client
- `ctx.std` — standard library (env, fs, process, io)
- `ctx.template` — template rendering (handlebars, jinja2, liquid)
- `ctx.tasks` — task registry (can add tasks dynamically)
- `ctx.traits[TraitType]` — mutable trait instances

Config execution can read environment variables, make HTTP requests, and perform other non-deterministic operations. The ordering of evaluation vs config execution (e.g., whether config runs before or after task files are evaluated) is an internal runtime detail that users must not depend on.

When multiple config sources set the same trait field, last-write-wins based on execution order.

### Task Execution

Task execution occurs when a user explicitly invokes a task (e.g., `aspect run <task>`). The runtime calls the task's `implementation` function with a `TaskContext` providing the full set of capabilities:

- `ctx.args` — parsed CLI arguments as declared by the task
- `ctx.bazel` — Bazel build, test, query, and info
- `ctx.std.fs` — filesystem operations (read, write, copy, rename, mkdir, etc.)
- `ctx.std.process` — subprocess execution
- `ctx.std.env` — environment variables, platform info, paths
- `ctx.std.io` — stdin/stdout/stderr streams
- `ctx.http()` — HTTP client (get, post, download with integrity checking)
- `ctx.template` — template rendering
- `ctx.traits[TraitType]` — frozen trait data (read-only, as configured)
- `ctx.task` — task metadata (name, group)

Task execution is inherently non-deterministic. It interacts with the outside world — building code, fetching URLs, writing files, running processes. The determinism guarantee applies only to evaluation; execution is where real work happens.

## AXL File Structure

AXL files (`.axl`) are Starlark files discovered in the `.aspect/` directory at the project root. The key file types are:

### Task Files

Any `.axl` file in `.aspect/` can define tasks. A task is declared at the top level using the `task()` function:

```python
def _impl(ctx: TaskContext) -> int:
    ctx.std.io.stdout.write("hello\n")
    return 0

my_task = task(
    name = "greet",           # optional, defaults to variable name
    group = ["utils"],        # CLI grouping
    implementation = _impl,   # the function to execute
    args = {                  # CLI argument declarations
        "name": args.string(default = "world"),
    },
    traits = [MyConfig],      # opt-in to trait types
)
```

Tasks define their own CLI arguments using the `args` module. Argument types include `args.string()`, `args.int()`, `args.uint()`, `args.boolean()`, their list variants, `args.positional()`, and `args.trailing_var_args()`. Each supports `required` and `default` parameters.

### Config File

`config.axl` defines a `config()` function that receives a `ConfigContext`. Config is where trait values are set and dynamic tasks can be registered:

```python
load("./my_trait.axl", "MyConfig")

def config(ctx: ConfigContext):
    cfg = ctx.traits[MyConfig]
    cfg.some_field = "value"
```

### Trait Definitions

Traits are global configuration objects shared across tasks that opt in. A trait type is defined using `trait()` with typed attributes:

```python
MyConfig = trait(
    message = attr(str, "default value"),
    count = attr(int, 1),
    callback = attr(typing.Callable[[str], str], lambda s: s),
)
```

Trait types are defined at evaluation time (pure). Trait instances are populated during config execution (mutable) and then frozen before being passed to task execution (read-only via `ctx.traits[MyConfig]`).

### Library Files

`.axl` files can define reusable functions and values that other files import via `load()`:

```python
load("./helpers.axl", "my_helper")
load("@demo//answer.axl", "ANSWER")
```

Load paths are resolved relative to the calling file's directory. The `@std` prefix (planned) will provide pure utility modules for use during evaluation.

## Async Model

Several APIs return `Future` objects for non-blocking operation:

- `ctx.http().get(...)` returns `Future[HttpResponse]`
- `ctx.http().download(...)` returns `Future`
- `ctx.bazel.build(...)` returns a `Build` object with `.wait()` and `.try_wait()`

Call `.block()` on a Future to wait for its result. This allows tasks to issue multiple requests concurrently before blocking on results.

## Bazel Integration

The Bazel API (`ctx.bazel`) provides structured access to Bazel operations:

- **build / test** — execute Bazel builds with flag passing, build event streaming, execution log capture, and workspace event monitoring
- **query** — chainable query builder (`ctx.bazel.query().targets("//...").deps().kind("rule").eval()`) returning `TargetSet`
- **info** — retrieve Bazel workspace info as a dict

Build event streaming supports late subscription (calling `.build_events()` after `.wait()` replays buffered events) and multiple concurrent iterators over the same build.
