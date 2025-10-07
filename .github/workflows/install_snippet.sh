#!/usr/bin/env bash

set -o errexit -o nounset -o pipefail

# Set by GH actions, see
# https://docs.github.com/en/actions/learn-github-actions/environment-variables#default-environment-variables
TAG=${GITHUB_REF_NAME}

cat <<EOF
## Install Aspect CLI

See full install instructions in [README.md](https://github.com/aspect-build/aspect-cli/blob/${TAG}/README.md#installation).

### Bazelisk (MacOS / Linux)

Configure [bazelisk](https://github.com/bazelbuild/bazelisk) to use the Aspect CLI for all developers in a repository
by adding the following to \`.bazeliskrc\` in the repository root:

\`\`\`sh
BAZELISK_BASE_URL=https://github.com/aspect-build/aspect-cli/releases/download
USE_BAZEL_VERSION=aspect/${TAG}
\`\`\`

> [!NOTE]
> This approach doesn't provide the \`aspect init\` command, which has to run outside a Bazel workspace.

The underlying version of Bazel can be configured in your \`.bazelversion\` file or the \`BAZEL_VERSION\` environment variable.

### Homebrew (MacOS)

To install the Aspect CLI on MacOS, you can run

\`\`\`sh
brew install aspect-build/aspect/aspect
\`\`\`

This installs the `aspect` command and also links it to `bazel`, just like the [bazelisk](https://github.com/bazelbuild/bazelisk) installer does.

EOF
