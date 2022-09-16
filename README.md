# aspect-cli

This is the source repository for the Aspect command-line interface for Bazel.
Check out our homepage at https://aspect.build

It's currently in pre-release.

Contact us at hello@aspect.dev if you'd like to discuss partnerships.

# Pre-commit install

https://pre-commit.com/#installation

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
