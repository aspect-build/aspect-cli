#!/usr/bin/env bash

set -o errexit -o nounset -o pipefail

# Set by GH actions, see
# https://docs.github.com/en/actions/learn-github-actions/environment-variables#default-environment-variables
TAG=${GITHUB_REF_NAME}

cat <<EOF
> [!WARNING]
> **Early Preview Release**<br>
> This is an early preview release of the Aspect CLI rewritten in Rust. It is under active development, and API changes are expected. We plan to stabilize the API and release a stable version in November 2025.

### Install Aspect CLI (MacOS and Linux)

\`\`\`sh
curl -fsSL https://install.aspect.build | bash
\`\`\`

### Install with Homebrew (MacOS only)

\`\`\`sh
brew install aspect-build/aspect/aspect
\`\`\`

**Documentation**: https://docs.aspect.build/cli/overview
**Additional installation instructions**: https://docs.aspect.build/cli/install
EOF
