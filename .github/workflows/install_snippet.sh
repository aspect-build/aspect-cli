#!/usr/bin/env bash

set -o errexit -o nounset -o pipefail

# TAG is provided by the calling workflow (e.g. "v2026.38.30", or
# "v2026.38.30-pre" for a pre-release). Strip the leading "v" to match the
# version string used in .aspect/version.axl. The install script and Homebrew
# only ever install the latest release, so a pre-release's notes offer the pin
# alone.
VERSION="${TAG#v}"

if [[ "${VERSION}" == *-pre ]]; then
    cat <<EOF
> [!WARNING]
> This is a pre-release for extended testing. It is not installed by
> \`install.aspect.build\`, Homebrew, or an unpinned launcher.

### Pin this pre-release in your repository

Create or update \`.aspect/version.axl\` at the root of your repository:

\`\`\`python
version("${VERSION}")
\`\`\`

See https://aspect.build/go/cli-version-pinning for the full reference.
EOF
    exit 0
fi

cat <<EOF
### Install Aspect CLI (MacOS and Linux)

\`\`\`sh
curl -fsSL https://install.aspect.build | bash
\`\`\`

### Install with Homebrew (MacOS only)

\`\`\`sh
brew install aspect-build/aspect/aspect
\`\`\`

### Pin this version in your repository

Create or update \`.aspect/version.axl\` at the root of your repository so
everyone (and CI) uses the same Aspect CLI version:

\`\`\`python
version("${VERSION}")
\`\`\`

See https://aspect.build/go/cli-version-pinning for the full reference.

**Documentation**: https://aspect.build/go/cli
**Additional installation instructions**: https://aspect.build/go/install
EOF
