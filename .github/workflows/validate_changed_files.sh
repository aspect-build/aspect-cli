#!/bin/bash

set -o errexit -o nounset -o pipefail

INVALID_FILE_PATHS=('pkg/plugin/sdk') # Array of filepaths that are not allowed
VALID_FILE_PATHS=('pkg/plugin/sdk/v1alpha2') # Array of filepaths that are allowed

# by default the only local branch will be pull/PR#/merge
# fetch only the latest commit from the 2 branches in question to avoid fetching the entire repo which could be costly
git fetch --depth 1 origin "$1"
git fetch --depth 1 origin "$2"

git diff "origin/$1..origin/$2" --name-only | while read -r file; do

    # check if filepath matches a valid path. If so move to the next change
    for valid_path in "${VALID_FILE_PATHS[@]}"; do
        if [[ "${file}" == "${valid_path}"* ]]; then
            continue 2
        fi
    done

    # check if filepath matches an invalid path
    for invalid_path in "${INVALID_FILE_PATHS[@]}"; do
        if [[ "${file}" == "${invalid_path}"* ]]; then
            echo "Branch contains changes to filepaths that are invalid"
            exit 1
        fi
    done
done