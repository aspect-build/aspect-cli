#!/usr/bin/env bash
# Help first-time contributors discover the githooks configuration, but don't force it on them.
# See https://github.com/aspect-build/rules_lint/blob/main/docs/formatting.md#using-a-locally-defined-hook
inside_work_tree=$(git rev-parse --is-inside-work-tree 2>/dev/null)

IFS='' read -r -d '' GITHOOKS_MSG <<"EOF"
    cat <<EOF
  It looks like the git config option core.hooksPath is not set.
  aspect-cli uses hooks stored in a version-controlled folder to run tools such as formatters.
  You can disable this warning by running:

    echo "common --workspace_status_command=" >> ~/.bazelrc

  To set up the hooks, please run:

    git config core.hooksPath tools/githooks
EOF

if [ "${inside_work_tree}" = "true" ] && [ "$EUID" -ne 0 ] && [ -z "$(git config core.hooksPath)" ]; then
    echo >&2 "${GITHOOKS_MSG}"
fi

# Bazel takes one status command, so the attribution keys are emitted from
# inside this script rather than replacing it (crates/aspect-cli/src/builtins/aspect/workspace_data.axl).
# The nag above goes to stderr, leaving stdout to carry only `KEY value` lines.
# `|| true`: an `aspect` missing from PATH would otherwise leave this script
# exiting non-zero, and Bazel fails the build when the status command does.
aspect setup workspace-data || true
