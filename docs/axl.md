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

**§12 Use traits to inject behavior**: Tasks are supposed to stay generic, eg build task should not know about Workflows deployments
but should provide necessary injection points via traits to allow external `feature()` to inject behavior. 

For instance build.axl task uses BazelTrait to allow features to inspect final form of `BazelRc` object to decide whether to add `--remote_header` 
for the configured RE or BES backend.


**§13 Ending a task early.** `return code` from the task's implementation is
the normal way out. From a nested helper, `ctx.std.process.exit(code, message)`
ends the task with `code` (0..=255) and no traceback: the message prints as an
`ERROR:` line, or `INFO:` for code 0, and `ctx.defer` callbacks still run. At
the top of `_impl`, where a `return` can reach,
`return TaskConclusion(exit_code = 1, message = ...)` renders the same way.
So does raising an error whose type was declared with `traceback = False`
(§15), which exits with code 1. Keep a plain `fail("...")` for bugs, where the
traceback is what you want.

```python
def _require_target(ctx: TaskContext, targets: list[str]):
    if not targets:
        ctx.std.process.exit(1, "Provide a target, e.g. `aspect build //...`.")

def _impl(ctx: TaskContext) -> int | TaskConclusion:
    if not ctx.args.targets:
        return TaskConclusion(exit_code = 1, message = "Provide a target, e.g. `aspect build //...`.")
```

Either shortcut skips whatever the body had yet to run. A status surface opened
through `phases.new` is still closed: the handle's post-task hook (§14) sends
the terminal update with the runtime's verdict and the message the task ended
on. A surface you wire without `phases` needs the same hook.

**§14 Task hooks.** `ctx.hooks` is shared by `config.axl`, every feature impl,
and the task body. `ctx.hooks.pre_task(fn)` runs `fn(ctx)` before the body;
`ctx.hooks.post_task(fn)` runs `fn(ctx, conclusion)` after it, however it
ended: a return, an `exit`, or an error. `conclusion` is the runtime's
`TaskConclusion` (`exit_code`, `text`, `flagged`, `message`). Order is
pre-task hooks, body, post-task hooks, `ctx.defer` callbacks, bookend, each
list in registration order. A pre-task hook that exits or fails stands in for
the body, which never runs; post-task hooks still see that conclusion. A
post-task hook that fails is reported as a warning and changes nothing.
Register pre-task hooks from `config.axl` or a feature; once the body has
started they are refused.

```python
def _keep_scratch_on_failure(ctx: TaskContext, outcome: TaskConclusion):
    if outcome.exit_code == 0:
        ctx.std.fs.remove_dir_all(work)
    else:
        print("kept " + work + " for inspection")

ctx.hooks.post_task(_keep_scratch_on_failure)
```

Use `ctx.defer` for cleanup that needs no knowledge of how the task ended;
use a post-task hook when the outcome matters.

**§15 Errors.** An error is a value. `error` is the root error type:
`error("message")` builds one, and `.type(...)` on any error type derives a
new one. Every error has three attributes: `message`, `cause` (the error that
led to it, or `None`) and `stacktrace` (where it was constructed, a list of
frames with `name`, `path`, `line` and `column`, outermost first). A derived
type adds its own fields and inherits its parent's.

```python
DeployError = error.type(fields = {
    "deployment": str,
    "attempt": field(int, default = 1),
})
DeployTimeout = DeployError.type(fields = {"after_ms": int})
NotLoggedIn = error.type(traceback = False)

e = DeployTimeout("no ack", deployment = "prod", after_ms = 30000, cause = previous)
```

- **Declare error types at module top level.** A type is named by the
  variable it is first assigned to, and that name is what `repr(e)`, type
  errors and tracebacks show. Export only the types callers need to tell
  apart (§11).
- **Fields** take a type, or `field(type, default = ...)` for an optional
  field, the same specs `record()` takes (`attr()` is for traits). The constructor takes the message
  first, positionally or as `message = `, then `cause =` and the fields by
  name, and checks each field against its type. `message`, `cause` and
  `stacktrace` are reserved, and a child cannot redeclare an inherited field.
- **Telling errors apart.** `type(e) == "error"` for every error.
  `isinstance(e, T)` holds when `e` was built from `T` or from any type
  derived from it, so `isinstance(e, DeployError)` is true for a
  `DeployTimeout`, and `isinstance(e, error)` is true for all of them. An
  annotation behaves the same way: `def retry(e: DeployError)` accepts a
  `DeployTimeout`. `str(e)` is the message; `repr(e)` shows the type and every
  field.
- **Raising.** `fail(e)` raises `e` itself. Its type, fields, `cause` and
  `stacktrace` survive, so a caller that catches it gets back the same value.
  Re-raise a caught error the same way: `fail(err)`. `fail` with anything
  other than a single error behaves as it always has.
- **How an escaping error renders.** A type's `traceback` decides what the
  user sees when an error of that type ends the task. `True` (the default)
  shows the traceback and `TypeName: message`: right for bugs and failures
  someone has to debug. `False` prints only the message as an `ERROR:` line
  and exits with code 1, as `ctx.std.process.exit(1, message)` would: right
  for expected refusals like a missing login. Post-task hooks, `ctx.defer`
  callbacks and `ASPECT_DEBUG=1` behave as for any exit (§13, §14). A child
  inherits its parent's setting unless it sets its own.

**Catching: `err, value`.** A failure becomes a value with `catch`, in one
of two forms. Both give a pair, error first: `(None, value)` on success,
`(err, None)` on failure. Test `if err:` before touching `value`; errors are
truthy.

```python
# A future: `catch(...)` right before `block()`.
err, resp = ctx.http().get(url = url).catch().block()
if err:
    warn(ctx.std, "upload skipped: " + err.message)
    return
use(resp.status)

# Any call: `catch(function, *args, **kwargs)` makes the call for you.
err, config = catch(_load_config, ctx, path, types = [ConfigError])
if err:
    config = DEFAULT_CONFIG
```

- **Name what you expect.** `fut.catch(DeployError)` and
  `catch(fn, types = [DeployError])` catch only errors of those types (and
  their subtypes); anything else still raises, unchanged. A bare `catch()`
  catches every failure, including bugs raised with a plain `fail`, so prefer
  naming the types where you can. `types` is `catch`'s own keyword; every other
  argument goes to `function`.
- **An exit is not caught.** `ctx.std.process.exit` inside the caught code
  still ends the task as asked. A raised error value, even one whose type has
  `traceback = False`, is caught like any other.
- **What arrives.** An error raised with `fail(e)` arrives as `e`. Any other
  failure arrives as a plain `error`: `message` is the failure, `cause` holds
  the underlying reasons one link at a time, and `stacktrace` points at where
  it was raised (for a future, the `block()` call).
- **`fut.catch()` must be the last call before `block()`**, and can be called
  once.
- **Types follow the value.** A future is typed with what it resolves to, e.g.
  `Future[HttpResponse]`, so `block()` is an `HttpResponse` and
  `catch().block()` is `tuple[error | None, HttpResponse | None]`. `catch(fn,
  ...)` is checked as the call `fn(...)` it makes, and its value is typed with
  `fn`'s return type. Editors and the static typechecker see these; the
  runtime does not narrow `value` after `if err:`.
- **Keep the pair together.** Nothing checks that `err` was tested before
  `value` was used; a forgotten check shows up as `None` has no attribute
  `...` at the use site. Unpack both names on one line and test `err` right
  after.

