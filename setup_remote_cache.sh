#!/usr/bin/env bash
#
# Fetch the external remote cache configuration and add it to the user's Bazel RC file.

set -o errexit -o nounset -o pipefail

ENDPOINT=$(aws ssm get-parameter --region us-west-2 --name "aw_external_cache_endpoint" --query Parameter.Value --output text)
AUTH_STRING=$(aws ssm get-parameter --region us-west-2 --with-decryption --name "aw_external_cache_auth_header" --query Parameter.Value --output text)
SELF_PATH=$(dirname "${BASH_SOURCE[0]:-"$(command -v -- "$0")"}")

echo -e "build --remote_cache=\"grpcs://${ENDPOINT}:8980\" --remote_header=\"Authorization=Basic ${AUTH_STRING}\" --remote_accept_cached --remote_upload_local_results" >"${SELF_PATH}/.aspect/bazelrc/remote-cache.bazelrc"
