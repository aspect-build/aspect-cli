# `aspect watch`

`watch` is novel.

```
$ aspect watch build <query | ... targets>
$ aspect watch test <query | ... targets>
$ aspect watch run <target> -- < ... args>
$ while aspect watch | read; do ... done
$ aspect watch //.aspect/tasks:lint.axl%watch
```

## Design goals
- Bazel separates out coverage, we want a better plan for that
- As with `build`, it'd be nice if we could directly supply queries to `test` rather than doing `query | xargs test` dances
