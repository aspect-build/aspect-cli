#!/usr/bin/env bash
inside_work_tree=$(git rev-parse --is-inside-work-tree 2>/dev/null)

# Encourage developers to setup githooks
IFS='' read -r -d '' GITHOOKS_MSG <<"EOF"
  It looks like the git config option core.hooksPath is not set.
  This repository uses hooks stored in githooks/ to run tools such as formatters.
  You can disable this warning by running:

    echo "common --workspace_status_command=" >> ~/.bazelrc

  To set up the hooks, please run:

    git config core.hooksPath githooks
EOF

if [ "${inside_work_tree}" = "true" ] && [ "$EUID" -ne 0 ] && [ -z "$(git config core.hooksPath)" ]; then
	echo >&2 "${GITHOOKS_MSG}"
fi
