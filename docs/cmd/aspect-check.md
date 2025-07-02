# `aspect check`

The `check` command exists to align with `ruff check`, which runs the active set of linters.

`check --fix` should apply automatic remediations if available.

## Design goals
- Linters should be as pluggable as possible (`rules_lint`)
- We want linters to be able to advertise fixes and to apply them automatically
- We want to base as much of this as possible on incremental build rules
