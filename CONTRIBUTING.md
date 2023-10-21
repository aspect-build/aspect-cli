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

1. Check with release engineer in silo about which release version of Aspect Pro should be used to
   determine the version here.

2. Push a `x.x.x` tag to the repository at the commit you want to release with the semver version
   from the prior step:

    ```
    git tag x.x.x
    git push origin $_
    ```

    > A `v` version prefix is intentionally _not_ included in the release tag so that the GitHub root
    > download archive `https://github.com/aspect-build/aspect-cli/releases/download` can be used for
    > Bazelisk installs using `.bazeliskrc`:
    >
    > ```
    > BAZELISK_BASE_URL=https://github.com/aspect-build/aspect-cli/releases/download
    > USE_BAZEL_VERSION=aspect/x.x.x
    > ```

3. Watch the automation run on GitHub actions (it's very slow, be prepared to wait)

4. For go module support, we also require a `v1.x.x` tag that corresponds to the `x.x.x` release
   tag. We version the go module as `v1.x.x` since consuming `v2+` go modules downstream adds
   undesirable complication. For now with CLI major releases as `5.x.x` the corresponding go module
   version should be `v1.5xx.x` with the patch version matching and the minor zero-padded to two digits.
   (e.g. `5.4.3` -> `v1.504.3`, `5.56.78` -> `v1.556.78`)

    When the CLI major version is bumped to 6, this mapping will need to be updated.

    ```
    git tag v1.5xx.x
    git push origin $_
    ```

5. Update Homebrew Formula

    Once the GitHub release is complete and Aspect CLI release artifacts are available for download,
    follow the [instructions](https://github.com/aspect-build/homebrew-aspect#updating-formulas-to-the-latest-release)
    in our [homebrew-aspect](https://github.com/aspect-build/homebrew-aspect) repository to update the
    Homebrew Formulas in the `aspect-build/aspect` tap.

## Test Homebrew Formula and Bottles

### Install and Configure `nginx`

Install `nginx`. On MacOS, run `brew install nginx`.

Change the `nginx` config so that it listens on part `8090`. By default, `nginx` will listen on
`localhost:8080`.

-   Find the location of your `nginx` config, run `nginx -t`.
-   Update the default server stanza to listen on `8090`. It should look like the following:

```
    server {
        listen       8090;
        server_name  localhost;
```

-   Restart `nginx`. Run `brew services restart nginx`.

### Build, Stage, and Install Aspect CLI with Homebrew

To verify that the built Homebrew formula and bottles build and install properly,
please run the following:

```sh
$ bazel run //release:verify_homebrew_artifacts
```

This will build the artifacts, copy the bottles to your local `nginx` webserver's
serving directory, create an `aspect-build/aspect` tap, copy the formula to the
tap, install the bottle for your system, and verify that the version from the
CLI matches the expected version.

NOTE: This is not a test target, because it will copy files to various locations on your local
machine. The default permissions for a test target do not allow this.

If you would like to perform the set up for this verification step without the assertions, you can
run the following (with or without the `--stamp` flag):

```sh
$ bazel run //release:stage_for_dev
```
