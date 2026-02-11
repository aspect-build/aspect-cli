# What is axel-f?

axel-f is a configuration library for the Aspect CLI. It hooks into the CLI's built-in tasks (`build`, `test`, and conditionally `lint`) to augment them for the Aspect Workflows environment.

It does not define its own tasks. Instead, it provides a `config.axl` that configures existing tasks with:

- **Environment discovery** - reads platform configuration from disk (`/etc/aspect/workflows/platform/`) to detect available services like remote cache endpoints, BES endpoints, and RBE configuration.
- **Service detection** - automatically detects and connects to services available during Aspect Workflows CI runs (e.g. deliveryd for artifact delivery, remote cache, build result storage).
- **Flag injection** - generates and injects bazelrc flags (startup flags, build flags) based on the detected environment, Bazel version, and host configuration.
- **CI integration** - when running in CI (GitHub Actions, Buildkite), integrates with GitHub APIs to post check runs, PR review comments, and lint suggestions with auto-fix support.
- **Lifecycle hooks** - hooks into task lifecycle events (build start/end, build events, delivery) to coordinate reporting and artifact management.

## How it fits in

axel-f is an external package, shipped with the CLI by default but resolved through `MODULE.aspect` like any other dependency. The architecture:

1. **Rust runtime** - Starlark interpreter + native APIs (`ctx.bazel`, `ctx.http()`, `ctx.std.*`)
2. **Builtins** (compiled into the binary) - `build`, `test`, `axl add`
3. **External packages** (via `MODULE.aspect`) - axel-f, `aspect_rules_lint`, etc.
4. **Customer config** (`.aspect/config.axl`, optional) - customer-specific overrides and customization

axel-f and packages like `aspect_rules_lint` are peers in the dependency graph. axel-f is the "batteries included" package that makes the built-in tasks work seamlessly in the Aspect Workflows environment.

## What it does NOT do

- Define core tasks (`build`, `test`, `lint`) - those are builtins or come from other packages
- Replace or wrap the Bazel command - the CLI invokes Bazel as a subprocess
- Require customer configuration - it works out of the box by auto-detecting the environment

---

# Config evaluation model

## Opt-in via use_config

Config evaluation from external packages is **never automatic**. A package having a `config.axl` file does nothing on its own. Two explicit declarations are required:

### Package side: declaring config availability

A package declares its config functions in its own `MODULE.aspect`:

```python
# In the package's MODULE.aspect
use_config("./config.axl", "config")
use_config("./lint_config.axl", "lint_config",
    requires = [("aspect_rules_lint", "^1.0.0")])
```

This says "I offer these config functions" but has no effect until the consumer enables them. A package can declare multiple `use_config()` directives for different config functions from the same or different files.

#### Conditional activation with `requires` and `conflicts`

`use_config()` supports two parameters for conditional activation based on the resolved dependency graph:

- **`requires`** - packages that must be present for this config to activate
- **`conflicts`** - packages that must be absent for this config to activate

If any condition is not met, the runtime **skips the entire file** - it is never loaded, so `load()` statements referencing conditional packages never execute.

`requires` accepts either bare strings (any version) or `(name, version_constraint)` tuples:

```python
requires = ["aspect_rules_lint"]                           # any version
requires = [("aspect_rules_lint", "^1.0.0")]               # version-constrained
requires = [("aspect_rules_lint", ">= 1.0.0, < 2.0.0")]   # explicit range
```

`conflicts` accepts bare package name strings:

```python
conflicts = ["aspect_rules_lint"]
```

Multiple entries in either list are AND-ed: all conditions must be satisfied.

#### Version constraint syntax

Version constraints follow Cargo/semver conventions:

| Syntax | Meaning | Example |
|---|---|---|
| `^1.0.0` | Compatible with version | `>= 1.0.0` and `< 2.0.0` |
| `~1.0.0` | Patch-level changes only | `>= 1.0.0` and `< 1.1.0` |
| `>= 1.0.0, < 2.0.0` | Explicit range | Comma-separated constraints |
| `= 1.5.0` | Exact version | Only `1.5.0` |
| `*` | Any version | Same as bare string in `requires` |

### Consumer side: enabling config

The root `MODULE.aspect` enables a package's config via `use_config = True`:

```python
# In the customer's MODULE.aspect
axl_archive_dep(
    name = "aspect_rules_lint",
    urls = [...],
    use_config = True,  # activate this package's config functions
)
```

Without `use_config = True`, the package's `use_config()` directives are ignored. The package's `.axl` files are still available for `load()` - only config evaluation is gated.

### axel-f is auto-enabled

axel-f ships with the CLI as the default configuration package. Its `use_config` is automatically enabled - customers do not need to set `use_config = True` for it. Customers can disable it if needed.

### Config activation is not transitive

If package A has `use_config = True` and package A depends on package B (which also declares `use_config()`), package B's config is **not** automatically enabled. The root `MODULE.aspect` must independently enable each package's config. This prevents transitive dependencies from silently injecting config behavior.

## Three phases

Config evaluation follows three strictly ordered phases:

### Phase 1: Resolution

`MODULE.aspect` is evaluated to produce the full dependency graph. All packages are fetched. `use_config()` directives are collected from each package's `MODULE.aspect`, but only those with `use_config = True` in the root are registered for phase 3. No `config.axl` code runs during this phase.

### Phase 2: Module loading

All `.axl` files across all resolved packages become available for `load()`. This is pure Starlark module evaluation - top-level code runs, functions and types are defined and cached. Config files registered in phase 1 are loaded as modules (their top-level code executes) but their config functions are **not** called. Config files whose `requires` or `conflicts` conditions are not satisfied are **not loaded at all**.

### Phase 3: Configuration pipeline

The runtime calls registered config functions in **topological dependency order** (leaves first, dependents later). Each config function receives the accumulated task state from all previous calls. The customer's `.aspect/config.axl` (if present) runs **last**, giving it full visibility and final say.

```
rules_lint config()  ->  axel-f config()  ->  .aspect/config.axl config()
   (leaf dep)             (depends on          (customer, always last)
                           rules_lint)
```

### Why this ordering works

- **Leaves first** means foundational packages (like `rules_lint`) establish defaults before packages that build on them (like `axel-f`).
- **Customer last** means customers can inspect and override anything any package has set.
- **Deterministic** - ordering is derived from the dependency graph via topological sort, so it doesn't change unless dependencies change.

## Cycles are prevented by design

`load()` statements inside config functions (phase 3) resolve against the module cache populated in phase 2. They never trigger another package's config evaluation. These are two separate concerns:

- `load("@aspect_rules_lint//lint/lint.axl", "Strategy")` -> fetches a cached module, returns types/functions
- `rules_lint`'s `config()` -> called by the runtime in the pipeline, not by `load()`

A config function can `load()` from its declared dependencies, but loading a module and evaluating its config are completely decoupled.

## Error handling

By default, a failing config function halts the pipeline with a clear error attributing the failure to the specific package. Packages can opt into best-effort mode, marking their config as optional so the pipeline continues if they fail.

## Config API

Each config function receives raw mutable access to task state via `ConfigContext`. This is the same model as today - `config()` iterates `ctx.tasks`, inspects and modifies task configs directly. There are no merge semantics or overwrite protection. The pipeline ordering (leaves first, customer last) provides the coordination model: later configs see and can override earlier ones.

## Conditional config: the lint task

The `lint` task is not a builtin - it comes from `@aspect_rules_lint`. axel-f configures lint (sets the strategy, changed files provider, GitHub integration) but only when the customer has `rules_lint` installed. This is the motivating example for the `requires` and `conflicts` parameters on `use_config()`.

### The problem

axel-f's lint configuration needs to:
1. `load("@aspect_rules_lint//lint/lint.axl", "StrategyHoldTheLine")` - import types from rules_lint
2. Set `task.config.strategy` and `task.config.changed_files_provider` on the lint task

Both fail if `rules_lint` isn't in the dependency graph. The `load()` is especially problematic because it's a top-level statement - it runs unconditionally at module load time (phase 2) before any config function is called.

Additionally, version compatibility matters: if axel-f loads types from `rules_lint`, the version the customer installed must be compatible with what axel-f was written against.

### The solution

axel-f splits its config into separate files and uses `requires`/`conflicts` to gate activation:

```python
# axel-f's MODULE.aspect
use_config("./config.axl", "config")

# When rules_lint IS present: configure lint with GitHub integration
use_config("./lint_config.axl", "lint_config",
    requires = [("aspect_rules_lint", "^1.0.0")])

# When rules_lint is NOT present: add helpful stub task
use_config("./lint_stub_config.axl", "lint_stub_config",
    conflicts = ["aspect_rules_lint"])
```

**`lint_config.axl`** contains the `load("@aspect_rules_lint//...")` statements and the lint-specific configuration. If `aspect_rules_lint` is not in the resolved dep graph, the runtime never loads this file - the `load()` never executes, no error occurs. The version constraint `^1.0.0` ensures compatibility when both packages are present.

**`lint_stub_config.axl`** activates only when `aspect_rules_lint` is absent (via `conflicts`). It registers a stub `lint` task that prints a helpful message directing the user to install `rules_lint`:

```python
def _lint_stub_impl(ctx):
    print("The lint task requires aspect_rules_lint.")
    print("Run: aspect axl add gh:aspect-build/rules_lint")
    return 1

lint = task(name = "lint", implementation = _lint_stub_impl)

def lint_stub_config(ctx):
    ctx.tasks.add(lint)
```

### How it plays out

| Customer has `rules_lint`? | `lint_config.axl` | `lint_stub_config.axl` | `aspect lint` behavior |
|---|---|---|---|
| Yes, at `^1.0.0` | Loaded and evaluated | Skipped (conflicts) | Full lint with GitHub integration |
| Yes, at `0.5.0` | Skipped (version mismatch) | Skipped (conflicts) | rules_lint's default lint, no axel-f hooks |
| No | Skipped (requires not met) | Loaded and evaluated | Helpful error with install instructions |

## Design constraints

- **Explicit opt-in on both sides** - packages declare config availability via `use_config()`, consumers enable it via `use_config = True`. Neither side alone is sufficient.
- **Not transitive** - only the root `MODULE.aspect` can enable config evaluation for a package. Intermediate dependencies cannot enable configs on behalf of their deps.
- **No config evaluation during phase 1** - MODULE.aspect resolution must complete fully before any config code runs.
- **load() never triggers config evaluation** - module loading and config evaluation are separate phases.
- **Topological ordering from dep graph** - no explicit ordering declarations needed. If you need your config to run after another package, declare a dependency on it.
- **Customer config always runs last** - `.aspect/config.axl` has final authority regardless of the dependency graph.
- **Full runtime access** - config functions have access to `ctx.std.process`, `ctx.http()`, `ctx.std.fs`, etc. Packages are trusted code.
- **Cargo-style version constraints** - `^`, `~`, comparison operators, comma-separated. Evaluated against the resolved dependency graph.
