# aspect-cli — repo-wide instructions for Claude Code

## AXL conventions

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

## Comments

- No banner-style comments (e.g. `# ---- section ----`).
- Keep comments minimal. no comment is the best comment. only comment when context is genuinely needed.
- Prefer documenting in task args / descriptions over inline comments.
