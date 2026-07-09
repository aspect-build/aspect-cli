# Guidelines for writing AXL code


**§1 Prefer typed constructions.** Use `record(f = field(Type, default = ...))`
for any stable set of named fields crossing a function/module boundary
(public APIs, trait callbacks, `data`-serialized values). Use
`enum("a", "b", ...)` for fields drawn from a fixed value set. Use
`field(T | None, default = None)` for optional values.

Dicts / bare `struct(...)` are fine only for: open-ended bags with
user-supplied keys, test fakes, and one-off internal scratch. When a
reader would have to guess field names or value sets, use a record + enum.

**§2 Always annotate.** Add parameter and return-type annotations on AXL
functions whenever the types are expressible (builtins, in-scope
records/enums). Skip only when the type genuinely can't be modeled (e.g.
an unmodeled callable union) or for test helpers taking fakes.

```python
def my_hook(ctx: TaskContext, info: ReproFixInfo) -> ReproFixSuggestion:
    ...
```

**§3 Traits are interfaces, not data carriers.** A trait is the bridge
between a task and the features that extend it (and between features
themselves) — it exposes hooks/callbacks that features inject behavior
into and tasks invoke. Do not use a trait to ferry plain data between
phases; carry data through the feature's own closure state or a
record, and keep the trait's surface to the callable interface.

**§4 Use `def namespace(*args, **kwargs) -> namespace(..)`*** for exporting multiple symbols from a file.


```python
def _fn():
    pass 

_CONST="HELLO"

lib = namespace(
    fn = _fn,
    CONSTANT=_CONST,
    sub = namespace(...)
)

```



```python
_FLAGS={
    "x": flags.string()
}

bazel = namespace(
    runnable = _fn,
    flags = namespace(build = _FLAGS)
)
```


**§5 Comments are there to give additional context that isn't derivable from code**

Keep your comments precise, do not comment if not needed, most of the time code is self explanatory.

- Keep module and function `"""comment"""` short, a single line at best.
- No banner-style comments (e.g. `# ---- section ----`).
- Prefer documenting features in `task` `args` / `descriptions` over inline comments.


**§6 ctx.defer(fn)** is your friend for post task cleanup. use it, design libraries that makes it possible to use it.

Its going to execute deferred functions even on a fatal starlark error.


**§7 ctx repetition** design libraries that requires ctx to be passed only once. 

Only use `.new()` pattern if there is a state to hold, for instance it does not make sense for a library
of static data and constants to have a `.new()` constructor

Good:

```python
load("./lib.axl", "github")

gh = github.new(ctx)

gh.list_pull_requests()
```

```python
load("./lib.axl", "github")

ph = phases.new(ctx)

ctx.defer(ph.teardown)
ph.setup()

# do some work
ph.report_progress()
```

Bad:

```python
load("./lib.axl", "github")

gh.list_pull_requests(ctx, param=1)
```

Also bad: 

```python
load("./lib.axl", "bazel")

x = bazel.new(ctx, trait) # bazel.new() can already access ctx.traits
```

**§8 Avoid Trait suffix on trait types**: `Bazel` is preferred over `BazelTrait`


§1 **§9 Design traits that are easy to understand**: Design as if its going to be used by advanced users, but easy to grasp for someone new.

```python
Bazel = trait(
    build_start = attr(typing.Callable[[]])
    build_end = attr(typing.Callable[[]])
)

```

**§10 Library design**: Check what exists first, extend/bend as needed. 

Design libraries that can be reused by aspect-cli users, make them so that they require minimal amount of imports
and leaks into the use site as little as possible. 

Use UPPERCASE name for constant that are exported from the libraries.

Bad 
```python
load("./lib.axl", "lib", "lib_setup", "LibResult", "lib_CONSTANT")

def impl():
   r: LibResult = lib_setup(param=lib_CONSTANT)
```

Good 
```python
load("./lib.axl", "lib")

def impl():
   ll = lib.new(ctx)
   r: lib.Result = ll.setup() #param is already default to lib.CONSTANT
   
```


bad 
```python
load("./lib.axl", "lib")

def impl():
   lib.resolve_flags()
```

good 
```python
load("./lib.axl", "lib")

def impl():
   lib.flags.resolve()
```

**§11 Visibility**: An internal code can still live in a public file but be private the outsiders

Avoid splitting code into `/private` vs public just because its not open to the public. Design apis
that are carefully promoted to the public api with minimal api.

Only export constants/functions/types if they are required in the public api.

Symbols can be exported via `testonly_` prefix for writing unit tests. 