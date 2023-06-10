#!/bin/bash

set -o errexit -o nounset -o pipefail

# Set by GH actions, see
# https://docs.github.com/en/actions/learn-github-actions/environment-variables#default-environment-variables
TAG=${GITHUB_REF_NAME}


cat << EOF
## Install [Aspect CLI](https://www.aspect.build/cli)

See full install instructions in [README.md](https://github.com/aspect-build/aspect-cli/blob/${TAG}/README.md).

### Homebrew (MacOS)

Link the [Aspect CLI](https://www.aspect.build/cli) as \`bazel\` just like the [bazelisk](https://github.com/bazelbuild/bazelisk) installer does:

\`\`\`
% brew install aspect-build/aspect/aspect
\`\`\`

### Bazelisk (MacOS / Linux)

Configure [bazelisk](https://github.com/bazelbuild/bazelisk) to use the [Aspect CLI](https://www.aspect.build/cli) for all developers. Add this to \`.bazeliskrc\` in your project folder:

\`\`\`
BAZELISK_BASE_URL=https://github.com/aspect-build/aspect-cli/releases/download
USE_BAZEL_VERSION=aspect/${TAG}
\`\`\`

The underlying version of Bazel can be configured in your \`.bazelversion\` file or the \`BAZEL_VERSION\` environment variable.

EOF
