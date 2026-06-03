# aspect-cli — repo-wide instructions for Claude Code

## AXL: prefer typed constructions

When adding new features to AXL code in this repo, prefer **records and
enums** over untyped `dict` / `struct` values. Reach for:

- `record(field1 = field(Type), field2 = field(Type, default = ...))`
  for any value with a stable set of named fields that crosses a
  function or module boundary. Public-API surfaces — anything users
  consume from `config.axl`, anything passed to a trait callback,
  anything serialized through `data` — should be records, not dicts.
- `enum("kind_a", "kind_b", ...)` for any field whose value is drawn
  from a small, fixed set. The enum validates inputs (typos raise
  with a clear error), documents the legal values at the definition
  site, and lets call sites compare against `.value` rather than
  ad-hoc string constants.
- `field(T | None, default = None)` for genuinely optional values, so
  the absence is explicit and typed rather than encoded as `""` or `0`.

Dicts and bare `struct(...)` literals are still appropriate for:

- Open-ended bags whose keys are user-supplied or genuinely
  heterogeneous (e.g. an `extras: dict` escape hatch on a record).
- Tests that fake a small slice of a public type and don't want to
  load the real record.
- One-off internal scratch data that doesn't cross a module
  boundary.

When in doubt: if a downstream reader would have to guess the field
names or their value sets, it should be a record + enum, not a dict.

## AXL: always annotate

When writing or editing AXL functions, **always add parameter and
return-type annotations** if the relevant types are available
(builtins like `str`, `int`, `list`, `dict`, or records/enums in
scope). Annotations:

- Validate inputs at the call site — typos and shape drift fail
  fast instead of corrupting state silently.
- Document the contract at the signature, so readers don't have to
  reverse-engineer it from the body.
- Compose with record/enum types: a parameter annotated `info:
  ReproFixInfo` immediately tells the reader which fields are
  available.

Example shape:

```python
def my_hook(ctx: TaskContext, info: ReproFixInfo) -> ReproFixSuggestion:
    ...
```

Skip an annotation only when the type genuinely isn't expressible
(e.g. a callable union the runtime doesn't model) or when a test
helper deliberately accepts a fake/struct in place of a real record.
