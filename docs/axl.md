# Guidelines for writing AXL code


**Prefer typed constructions.** Use `record(f = field(Type, default = ...))`
for any stable set of named fields crossing a function/module boundary
(public APIs, trait callbacks, `data`-serialized values). Use
`enum("a", "b", ...)` for fields drawn from a fixed value set. Use
`field(T | None, default = None)` for optional values.

Dicts / bare `struct(...)` are fine only for: open-ended bags with
user-supplied keys, test fakes, and one-off internal scratch. When a
reader would have to guess field names or value sets, use a record + enum.

**Always annotate.** Add parameter and return-type annotations on AXL
functions whenever the types are expressible (builtins, in-scope
records/enums). Skip only when the type genuinely can't be modeled (e.g.
an unmodeled callable union) or for test helpers taking fakes.

```python
def my_hook(ctx: TaskContext, info: ReproFixInfo) -> ReproFixSuggestion:
    ...
```

**Traits are interfaces, not data carriers.** A trait is the bridge
between a task and the features that extend it (and between features
themselves) — it exposes hooks/callbacks that features inject behavior
into and tasks invoke. Do not use a trait to ferry plain data between
phases; carry data through the feature's own closure state or a
record, and keep the trait's surface to the callable interface.

**Use `def namespace(*args, **kwargs) -> namespace(..)`*** for exporting multiple symbols from a file.


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


**Comments are there to give additional context that isn't derivable from code**

Keep your comments precise, do not comment if not needed, most of the time code is self explanatory.

- Keep module and function `"""comment"""` short, a single line at best.
- No banner-style comments (e.g. `# ---- section ----`).
- Prefer documenting features in `task` `args` / `descriptions` over inline comments.


**ctx.defer(fn)** is your friend for post task cleanup. use it, design libraries that makes it possible to use it.


**ctx repetition** design libraries that requires ctx to be passed only once. 

Preferred method: 

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

Unpreferred method: 

```python
load("./lib.axl", "github")

gh.list_pull_requests(ctx, param=1)
```

**Avoid Trait suffix on trait types**: `Bazel` is preferred over `BazelTrait`


**Design traits that are easy to understand**: Design as if its going to be used by advanced users, but easy to grasp for someone new.

```python
Bazel = trait(
    build_start = attr(typing.Callable[[]])
    build_end = attr(typing.Callable[[]])
)

```

**Do not design alternative libraries**: Check what exists first, extend/bend as needed.