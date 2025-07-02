# `aspect init`

The init command exists to apply templates and create repositories.

`cargo init`, `git init` and `rails new` are all paradigm cases.

## Design goals
- Initialized repositories must record the flags they were initialized with
- We want a path for _upgrading_ initialized repositories
- Initialized repositories should conform to the latest of our recommended practices

