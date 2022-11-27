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

## Releasing

1. Push a `x.x.x` tag to the repository at the commit you want to release with the semver version
   you want to use for the release:

    ```
    git tag x.x.x
    git push origin x.x.x
    ```

    > A `v` version prefix is intentionally _not_ included in the release tag so that the GitHub root
    > download archive `https://github.com/aspect-build/aspect-cli/releases/download` can be used for
    > Bazelisk installs using `.bazeliskrc`:
    >
    > ```
    > BAZELISK_BASE_URL=https://github.com/aspect-build/aspect-cli/releases/download
    > USE_BAZEL_VERSION=aspect/x.x.x
    > ```

2. For go module support, we also require a `v1.x.x` tag that corresponds to the `x.x.x` release
   tag. We version the go module as `v1.x.x` since consuming `v2+` go modules downstream adds
   undesirable complication. For now with CLI major releases as `5.x.x` the corresponding go module
   version should be `v1.x.x` with the minor & patch versions matching. When the CLI major version
   is bumped to 6, this mapping will need to be updated.

    ```
    git tag v1.x.x
    git push origin v1.x.x
    ```

3. Watch the automation run on GitHub actions
