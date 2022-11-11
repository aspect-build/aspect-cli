Want to contribute? Great!

When you send a PR, our Contributor License Agreement needs to be signed, which is checked by the
CLA bot, thanks to SAP for this service.

## Formatting/linting

We suggest using a pre-commit hook to automate this. First
[install pre-commit](https://pre-commit.com/#installation), then run

```shell
pre-commit install
pre-commit install --hook-type commit-msg
```

Otherwise the CI will yell at you about formatting/linting violations.

# Dev Workflow

To make changes to your go files, run the following to fix your dependencies and build files:

`bazel run tidy`

To run a command using the Bazel managed version of go, use the following pattern:

`bazel run go -- ` followed by your go command

For instance, to view the version of go, you execute the following:

`bazel run go -- version`

## Resolving merge conflicts on go.mod, go.sum and deps.bzl

If you have not made any custom changes to these files but have conflicts in your merge, then you can run:

git fetch origin --prune
git checkout origin/main -- go.mod go.sum deps.bzl

`bazel run //:tidy`

This will fetch go.mod, go.sum and deps.bzl from origin/main and replace your files with them.
Then, go tidy and gazelle will be run over these files to update them.
After this, you should be able to merge your changes without any conflicts in those files.
