#!/usr/bin/env bash
# Common CLI bootstrap for all CI platforms (Buildkite, GitHub Actions).
# Source this file to configure Bazel opts, check runner health, and build //:cli.
#
# Exports: BAZEL_STARTUP_OPTS, BAZEL_BUILD_OPTS, DISABLE_PLUGINS_FLAG
set -eu

# Build remote cache/BES flags from ASPECT_WORKFLOWS_* env vars injected by the runner.
BAZEL_REMOTE_FLAGS=""
[ -n "${ASPECT_WORKFLOWS_BES_BACKEND:-}" ]                  && BAZEL_REMOTE_FLAGS="${BAZEL_REMOTE_FLAGS} --bes_backend=${ASPECT_WORKFLOWS_BES_BACKEND}"
[ -n "${ASPECT_WORKFLOWS_BES_RESULTS_URL:-}" ]              && BAZEL_REMOTE_FLAGS="${BAZEL_REMOTE_FLAGS} --bes_results_url=${ASPECT_WORKFLOWS_BES_RESULTS_URL}"
[ -n "${ASPECT_WORKFLOWS_REMOTE_CACHE:-}" ]                 && BAZEL_REMOTE_FLAGS="${BAZEL_REMOTE_FLAGS} --remote_cache=${ASPECT_WORKFLOWS_REMOTE_CACHE}"
[ -n "${ASPECT_WORKFLOWS_REMOTE_BYTESTREAM_URI_PREFIX:-}" ] && BAZEL_REMOTE_FLAGS="${BAZEL_REMOTE_FLAGS} --remote_bytestream_uri_prefix=${ASPECT_WORKFLOWS_REMOTE_BYTESTREAM_URI_PREFIX}"

BAZEL_STARTUP_OPTS=""
DISABLE_PLUGINS_FLAG=""

if [ -n "${ASPECT_WORKFLOWS_RUNNER:-}" ]; then
  echo "Running on Aspect Workflows runner"
  STORAGE_PATH="${ASPECT_WORKFLOWS_RUNNER_STORAGE_PATH:-/mnt/ephemeral}"

  # Derive repo name from the CI platform's git remote URL.
  # Mirrors _parse_git_url_name + _sanitize_filename in environment.axl.
  REPO_NAME=""
  if [ -n "${BUILDKITE_REPO:-}" ]; then
    REPO_NAME=$(echo "${BUILDKITE_REPO}" | sed 's|/*$||' | sed 's|\.git$||' | sed 's|.*/||' | sed 's|.*:||' | sed 's|[^a-zA-Z0-9._-]|_|g')
  elif [ -n "${GITHUB_REPOSITORY:-}" ]; then
    REPO_NAME=$(echo "${GITHUB_REPOSITORY}" | sed 's|.*/||' | sed 's|[^a-zA-Z0-9._-]|_|g')
  fi

  # Derive workspace subdir from the checkout path.
  WORKSPACE_DIR="${BUILDKITE_BUILD_CHECKOUT_PATH:-${GITHUB_WORKSPACE:-$(pwd)}}"
  SUBDIR=$(basename "${WORKSPACE_DIR}" | sed 's|[^a-zA-Z0-9._-]|_|g')

  if [ -n "${REPO_NAME}" ]; then
    OUTPUT_USER_ROOT="${STORAGE_PATH}/bazel/${REPO_NAME}/${SUBDIR}"
    OUTPUT_BASE="${STORAGE_PATH}/output/${REPO_NAME}/${SUBDIR}"
  else
    OUTPUT_USER_ROOT="${STORAGE_PATH}/bazel/${SUBDIR}"
    OUTPUT_BASE="${STORAGE_PATH}/output/${SUBDIR}"
  fi

  BAZEL_STARTUP_OPTS="--nohome_rc --nosystem_rc --output_user_root=${OUTPUT_USER_ROOT} --output_base=${OUTPUT_BASE}"
  BAZEL_REMOTE_FLAGS="${BAZEL_REMOTE_FLAGS} --repository_cache=${STORAGE_PATH}/caches/repository"

  if [ -z "${ASPECT_WORKFLOWS_RUNNER_NO_LEGACY_CLI:-}" ]; then
    DISABLE_PLUGINS_FLAG="--aspect:disable_plugins"
  fi
fi

export BAZEL_STARTUP_OPTS
export BAZEL_BUILD_OPTS="--config=ci --announce_rc ${BAZEL_REMOTE_FLAGS}"
export DISABLE_PLUGINS_FLAG

echo "Startup opts: ${BAZEL_STARTUP_OPTS}"
echo "Build opts: ${BAZEL_BUILD_OPTS}"

if [ -f /etc/bazel.bazelrc ]; then
  echo "/etc/bazel.bazelrc exists ($(wc -l < /etc/bazel.bazelrc) lines)"
else
  echo "/etc/bazel.bazelrc does not exist"
fi

# Check for a stale Bazel lock before doing any work, so we detect unhealthy runners
# early rather than letting a build hang indefinitely.
if [ -n "${ASPECT_WORKFLOWS_RUNNER:-}" ]; then
  echo "Checking for bazel lock..."
  # We use a short timeout so we capture the "Another command (pid=X)" message without
  # blocking indefinitely.
  # shellcheck disable=SC2086
  LOCK_OUTPUT=$(timeout 5 bazel $BAZEL_STARTUP_OPTS info 2>&1) || true
  BUSY_PID=$(echo "$LOCK_OUTPUT" | grep -o '(pid=[0-9]*)' | grep -o '[0-9]*') || true
  if [ -n "$BUSY_PID" ]; then
    echo "Bazel is locked by pid=${BUSY_PID}, signalling unhealthy"
    /etc/aspect/workflows/bin/signal_instance_unhealthy
    exit 78
  fi
fi

echo "--- Building aspect-cli"
# We intentionally call 'bazel' here instead of 'aspect' for two reasons:
#
#  1. On Aspect Workflows runners, 'bazel' is the previous stable release of
#     aspect-cli, so it is guaranteed not to be broken by this PR.
#
#  2. On PRs, the 'aspect' binary produced by this repo is exactly what we're
#     trying to test. Using it to build itself would mean a PR that introduces
#     a fatal startup bug could never bootstrap — masking the real failure.
#     Using the stable 'bazel' wrapper avoids this bootstrapping circularity.
#
# -c dbg keeps the binary unstripped so stack traces are readable.
# --remote_download_toplevel avoids fetching intermediate artifacts from cache.
# shellcheck disable=SC2086
bazel $DISABLE_PLUGINS_FLAG $BAZEL_STARTUP_OPTS build $BAZEL_BUILD_OPTS \
  -c dbg --remote_download_toplevel --show_progress_rate_limit=1 //:cli

# In GitHub Actions, env vars do not persist across steps unless written here.
if [ -n "${GITHUB_ENV:-}" ]; then
  {
    echo "BAZEL_STARTUP_OPTS=${BAZEL_STARTUP_OPTS}"
    echo "BAZEL_BUILD_OPTS=${BAZEL_BUILD_OPTS}"
    echo "DISABLE_PLUGINS_FLAG=${DISABLE_PLUGINS_FLAG}"
  } >> "${GITHUB_ENV}"
fi
