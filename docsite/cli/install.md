
You can install the Aspect CLI in two ways: the launcher is a small binary that's added to your PATH and knows how to run the correct version of the much larger Aspect Workflows binary. This is similar to how `nvm` or `n-` can run a version of NodeJS on your PATH, or how a Bazel wrapper can run the correct version of Bazel for a project.

## Install the Aspect CLI with `direnv` and `multitool`

This approach assumes your development environment follows Aspect's recommended setup, utilizing `bazel_env.bzl`. For reference, see the Starter repositories at [bazel-starters on GitHub](https://github.com/bazel-starters), which demonstrate this configuration.

1. Add `aspect` to the multitool lockfile, as shown in [this example](https://github.com/bazel-starters/shell/blob/main/tools/tools.lock.json).
2. Build and run your `bazel_env` target. Bazel handles the installation of `aspect`, ensuring it's available on your `PATH`.

For a deeper dive into this pattern, watch the Aspect Insights podcast episode titled "Developer Tooling in Monorepos with `bazel_env`":
<iframe
  className="w-full aspect-video rounded-xl"
  src="https://www.youtube.com/embed/TDyUvaXaZrc"
  title="YouTube video player"
  frameBorder="0"
  allow="accelerometer; autoplay; clipboard-write; encrypted-media; gyroscope; picture-in-picture"
  allowFullScreen
></iframe>

## If you use GitHub Actions, you can install the Aspect CLI with the `aspect-build/setup-aspect-cli` action:

<Warning>
Future releases might use a shorter syntax, like:
```
uses: aspect-build/aspect-cli@2025.42.8
```
</Warning>

Install the Aspect CLI onto the `PATH` in a workflow step. Here's an example pattern:

```yaml
jobs:
  my-job:
    runs-on: ubuntu-latest
    steps:
      - uses: actions/checkout@v5
      - uses: bazel-contrib/setup-bazel@0.15.0
      - name: Install Aspect CLI
        uses: jaxxstorm/action-install-gh-release@v2.1.0
        with:
          repo: aspect-build/aspect-cli
          tag: 2025.42.8 # Check for newer releases, or change to 'latest'
          asset-name: aspect-cli
          platform: unknown_linux
          arch: x86_64
          extension-matching: disable
          rename-to: aspect
          chmod: 0755

      # Subsequent steps can just execute `aspect`
```

## Install the Aspect CLI manually from GitHub

Access the [Aspect CLI Releases page on GitHub](https://github.com/aspect-build/aspect-cli/releases) repository to download the appropriate binary for your platform.

### macOS

1. Download the `aspect-cli_aarch64_apple_darwin` binary from the [Aspect CLI Releases page](https://github.com/aspect-build/aspect-cli/releases).
2. Open your terminal and run the following commands to clear the untrusted developer bit, make the binary executable, and move it to your `PATH`:

  ```shell
  xattr -c ~/Downloads/aspect-cli_aarch64_apple_darwin
  chmod u+x ~/Downloads/aspect-cli_aarch64_apple_darwin
  sudo mv ~/Downloads/aspect-cli_aarch64_apple_darwin /usr/local/bin/aspect
  ```

<Danger>
Installation via Homebrew isn't available for the latest CLI releases. Please check the [Aspect CLI Releases page](https://github.com/aspect-build/aspect-cli/releases) for updates and compatibility.
</Danger>