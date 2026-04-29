# Testing lint violations

This example uses [ShellCheck](https://www.shellcheck.net/) to lint `hello.sh`.

## Run lint locally

```sh
cd examples/lint
aspect lint
```

## Producing new violations

Add shell code that violates a ShellCheck rule. Common examples:

**SC2086 — unquoted variable (word splitting / globbing)**
```sh
echo $var          # bad
echo "$var"        # good
```

**SC2164 — `cd` without error handling**
```sh
cd /some/dir       # bad
cd /some/dir || exit  # good
```

**SC2046 — unquoted command substitution**
```sh
echo $(ls)         # bad
echo "$(ls)"       # good
```

**SC2181 — checking `$?` instead of the command directly**
```sh
some_cmd; if [ $? -ne 0 ]; then ...   # bad
if ! some_cmd; then ...               # good
```

## Testing the GitHub PR comment flow

The `GithubLintComments` feature only posts review comments on lines that appear
in the PR diff. To verify end-to-end posting:

1. Create a PR branch.
2. Add (or modify) a line in `hello.sh` that contains a ShellCheck violation.
3. Push — the changed line will be in the diff and the comment will be posted.

A violation on a line that was **not** changed in the PR will be silently
filtered (`filtered by diff: N` in the debug summary) because GitHub's review
comment API rejects comments outside the diff.
