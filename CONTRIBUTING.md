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

1. Sync your local `main` branch to the commit to release from.
   Run `node ./tools/workspace_status.js | grep STABLE_ASPECT_CLI_BAZELISK_COMPAT_VERSION`
   to determine the current release version. This version follows our monorepo versioning scheme minus
   the hash so that it is compatible with Bazelisk. See comment in `workspace_status.sh` for more info.

2. Push a `xxxx.x.x` tag to the repository at the commit you want to release with the semver version
   from the prior step:

    ```
    git tag xxxx.x.x
    git push origin $_
    ```

    > A `v` version prefix is intentionally _not_ included in the release tag so that the GitHub root
    > download archive `https://github.com/aspect-build/aspect-cli/releases/download` can be used for
    > Bazelisk installs using `.bazeliskrc`:
    >
    > ```
    > BAZELISK_BASE_URL=https://github.com/aspect-build/aspect-cli/releases/download
    > USE_BAZEL_VERSION=aspect/xxxx.x.x
    > ```

3. Watch the automation run on GitHub actions (it's very slow, be prepared to wait)

4. For go module support, we also require a `v1.yyyyww.x` tag that corresponds to the `yyyy.ww.x` release
   tag. We version the go module as `v1.x.x` since consuming `v2+` go modules downstream adds
   undesirable complication. The week should be zero-padded to two digits.

    For example, the version `2024.1.1` maps to `v1.202401.1` and `2024.39.4` maps to `v1.202439.4`.

    ```
    git tag v1.yyyyww.x
    git push origin $_
    ```
