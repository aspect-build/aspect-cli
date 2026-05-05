#!/usr/bin/env bash

set -o errexit -o nounset -o pipefail

# TAG is provided by the calling workflow (e.g. "v2026.19.2"). Strip the
# leading "v" to match the version string used in .aspect/version.axl.
VERSION="${TAG#v}"

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

See https://docs.aspect.build/cli/version-pinning for the full reference.

**Documentation**: https://docs.aspect.build/cli/overview
**Additional installation instructions**: https://docs.aspect.build/cli/install
EOF
