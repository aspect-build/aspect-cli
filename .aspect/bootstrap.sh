#!/usr/bin/env bash
# Common CLI bootstrap for all CI platforms (Buildkite, GitHub Actions, CircleCI, GitLab).
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

# --build_metadata flags for the pre-build invocation. Only set when we're
# forwarding events to a BES backend (the Aspect Web UI or similar) —
# otherwise the metadata has nowhere to surface.
#
# Mirrors the aspect-cli metadata pipeline (crates/aspect-cli/src/builtins/
# aspect/lib/build_metadata.axl) but simplified for the bootstrap step —
# we want the Web UI invocation page to show who/what/where without
# requiring a full aspect-cli bring-up.
#
# Uses a bash array to preserve spaces in values (e.g. "Bootstrap CLI",
# `Greg Magolan <greg@aspect.build>`). The ${ARR[@]+"${ARR[@]}"} expansion
# at the invocation site keeps this set -u-safe even when empty.
METADATA_ARGS=()
if [ -n "${ASPECT_WORKFLOWS_BES_BACKEND:-}" ]; then
  METADATA_ARGS+=("--build_metadata=ASPECT_TASK_NAME=Bootstrap CLI")
  METADATA_ARGS+=("--build_metadata=ASPECT_TASK_ID=bootstrap-cli")

  # ---- Git commit info (authoritative where available). ----
  # Single git call; splits on newlines. Fields: SHA, author name, author
  # email, subject, author date (ISO 8601). Skipped silently when we're
  # not inside a git repo (rare on CI but defensible).
  if git rev-parse --git-dir >/dev/null 2>&1; then
    GIT_INFO=$(git show HEAD --no-patch --format='%H%n%aN%n%aE%n%s%n%aI' 2>/dev/null || true)
    if [ -n "${GIT_INFO}" ]; then
      GIT_SHA=$(printf '%s\n' "${GIT_INFO}" | sed -n '1p')
      GIT_AUTHOR_NAME=$(printf '%s\n' "${GIT_INFO}" | sed -n '2p')
      GIT_AUTHOR_EMAIL=$(printf '%s\n' "${GIT_INFO}" | sed -n '3p')
      GIT_SUBJECT=$(printf '%s\n' "${GIT_INFO}" | sed -n '4p')
      GIT_DATE=$(printf '%s\n' "${GIT_INFO}" | sed -n '5p')
      [ -n "${GIT_SHA}" ]          && METADATA_ARGS+=("--build_metadata=COMMIT_SHA=${GIT_SHA}")
      [ -n "${GIT_AUTHOR_NAME}" ]  && METADATA_ARGS+=("--build_metadata=COMMIT_AUTHOR_NAME=${GIT_AUTHOR_NAME}")
      [ -n "${GIT_AUTHOR_EMAIL}" ] && METADATA_ARGS+=("--build_metadata=COMMIT_AUTHOR_EMAIL=${GIT_AUTHOR_EMAIL}")
      if [ -n "${GIT_AUTHOR_NAME}" ] && [ -n "${GIT_AUTHOR_EMAIL}" ]; then
        METADATA_ARGS+=("--build_metadata=COMMIT_AUTHOR=${GIT_AUTHOR_NAME} <${GIT_AUTHOR_EMAIL}>")
      fi
      [ -n "${GIT_SUBJECT}" ]      && METADATA_ARGS+=("--build_metadata=COMMIT_MESSAGE=${GIT_SUBJECT}")
      [ -n "${GIT_DATE}" ]         && METADATA_ARGS+=("--build_metadata=COMMIT_TIMESTAMP=${GIT_DATE}")
    fi
  fi

  # ---- CI-specific metadata. ----
  if [ -n "${BUILDKITE:-}" ]; then
    METADATA_ARGS+=("--build_metadata=CI_HOST=BUILDKITE")
    [ -n "${BUILDKITE_BRANCH:-}" ]        && METADATA_ARGS+=("--build_metadata=BRANCH_NAME=${BUILDKITE_BRANCH}")
    [ -n "${BUILDKITE_BUILD_CREATOR:-}" ] && METADATA_ARGS+=("--build_metadata=USER=${BUILDKITE_BUILD_CREATOR}")
    # Step-deep URL matches the status-check CI link.
    if [ -n "${BUILDKITE_BUILD_URL:-}" ]; then
      if [ -n "${BUILDKITE_STEP_ID:-}" ]; then
        METADATA_ARGS+=("--build_metadata=BUILD_URL=${BUILDKITE_BUILD_URL}/steps/canvas?sid=${BUILDKITE_STEP_ID}")
      else
        METADATA_ARGS+=("--build_metadata=BUILD_URL=${BUILDKITE_BUILD_URL}")
      fi
    fi
    # Repo info from BUILDKITE_REPO — handles SSH (git@host:owner/repo.git)
    # and HTTPS (https://host/owner/repo.git) forms.
    if [ -n "${BUILDKITE_REPO:-}" ]; then
      BK_HOST=""; BK_PATH=""
      BK_REST="${BUILDKITE_REPO#*://}"
      if [ "${BK_REST}" = "${BUILDKITE_REPO}" ]; then
        # No scheme → SSH form.
        BK_REST="${BUILDKITE_REPO#*@}"
        BK_HOST="${BK_REST%%:*}"
        BK_PATH="${BK_REST#*:}"
      else
        # Strip creds if any.
        case "${BK_REST}" in *@*/*) BK_REST="${BK_REST#*@}" ;; esac
        BK_HOST="${BK_REST%%/*}"
        BK_PATH="${BK_REST#*/}"
      fi
      BK_PATH="${BK_PATH%.git}"
      BK_OWNER="${BK_PATH%%/*}"
      BK_NAME="${BK_PATH##*/}"
      if [ -n "${BK_OWNER}" ] && [ -n "${BK_NAME}" ] && [ "${BK_OWNER}" != "${BK_NAME}" ]; then
        METADATA_ARGS+=("--build_metadata=REPO_OWNER=${BK_OWNER}")
        METADATA_ARGS+=("--build_metadata=REPO_NAME=${BK_NAME}")
        if [ -n "${BK_HOST}" ]; then
          METADATA_ARGS+=("--build_metadata=REPO_URL=https://${BK_HOST}/${BK_OWNER}/${BK_NAME}")
          case "${BK_HOST}" in
            *github*)    METADATA_ARGS+=("--build_metadata=VCS=GITHUB") ;;
            *gitlab*)    METADATA_ARGS+=("--build_metadata=VCS=GITLAB") ;;
            *bitbucket*) METADATA_ARGS+=("--build_metadata=VCS=BITBUCKET") ;;
          esac
        fi
      fi
    fi
    # RUN_TYPE + PR info.
    if [ -n "${BUILDKITE_PULL_REQUEST:-}" ] && [ "${BUILDKITE_PULL_REQUEST}" != "false" ]; then
      METADATA_ARGS+=("--build_metadata=RUN_TYPE=PULL_REQUEST")
      METADATA_ARGS+=("--build_metadata=PR_NUMBER=${BUILDKITE_PULL_REQUEST}")
      METADATA_ARGS+=("--build_metadata=PR_ID=${BUILDKITE_PULL_REQUEST}")
      [ -n "${BUILDKITE_BRANCH:-}" ]                    && METADATA_ARGS+=("--build_metadata=PR_SOURCE_BRANCH_NAME=${BUILDKITE_BRANCH}")
      [ -n "${BUILDKITE_PULL_REQUEST_BASE_BRANCH:-}" ]  && METADATA_ARGS+=("--build_metadata=PR_TARGET_BRANCH_NAME=${BUILDKITE_PULL_REQUEST_BASE_BRANCH}")
    elif [ -n "${BUILDKITE_TAG:-}" ]; then
      METADATA_ARGS+=("--build_metadata=RUN_TYPE=TAG")
      METADATA_ARGS+=("--build_metadata=TAG=${BUILDKITE_TAG}")
    elif [ "${BUILDKITE_SOURCE:-}" = "schedule" ]; then
      METADATA_ARGS+=("--build_metadata=RUN_TYPE=SCHEDULED")
    elif [ "${BUILDKITE_SOURCE:-}" = "ui" ] || [ "${BUILDKITE_SOURCE:-}" = "api" ] || [ "${BUILDKITE_SOURCE:-}" = "manual" ]; then
      METADATA_ARGS+=("--build_metadata=RUN_TYPE=MANUAL")
    else
      METADATA_ARGS+=("--build_metadata=RUN_TYPE=BRANCH_PUSH")
    fi

  elif [ -n "${GITHUB_ACTIONS:-}" ]; then
    METADATA_ARGS+=("--build_metadata=CI_HOST=GITHUB_ACTIONS")
    METADATA_ARGS+=("--build_metadata=VCS=GITHUB")
    [ -n "${GITHUB_ACTOR:-}" ] && METADATA_ARGS+=("--build_metadata=USER=${GITHUB_ACTOR}")
    # Repo info from GITHUB_REPOSITORY ("owner/repo").
    if [ -n "${GITHUB_REPOSITORY:-}" ]; then
      GH_OWNER="${GITHUB_REPOSITORY%%/*}"
      GH_NAME="${GITHUB_REPOSITORY##*/}"
      GH_SERVER="${GITHUB_SERVER_URL:-https://github.com}"
      METADATA_ARGS+=("--build_metadata=REPO_OWNER=${GH_OWNER}")
      METADATA_ARGS+=("--build_metadata=REPO_NAME=${GH_NAME}")
      METADATA_ARGS+=("--build_metadata=REPO_URL=${GH_SERVER}/${GH_OWNER}/${GH_NAME}")
      if [ -n "${GITHUB_RUN_ID:-}" ]; then
        METADATA_ARGS+=("--build_metadata=BUILD_URL=${GH_SERVER}/${GITHUB_REPOSITORY}/actions/runs/${GITHUB_RUN_ID}")
      fi
    fi
    # BRANCH_NAME: PR source branch (GITHUB_HEAD_REF) or refs/heads/<name>.
    if [ -n "${GITHUB_HEAD_REF:-}" ]; then
      METADATA_ARGS+=("--build_metadata=BRANCH_NAME=${GITHUB_HEAD_REF}")
    elif [ -n "${GITHUB_REF:-}" ]; then
      case "${GITHUB_REF}" in
        refs/heads/*) METADATA_ARGS+=("--build_metadata=BRANCH_NAME=${GITHUB_REF#refs/heads/}") ;;
        refs/tags/*)  METADATA_ARGS+=("--build_metadata=TAG=${GITHUB_REF#refs/tags/}") ;;
      esac
    fi
    # RUN_TYPE from GITHUB_EVENT_NAME; PR details from GITHUB_REF.
    case "${GITHUB_EVENT_NAME:-}" in
      pull_request|pull_request_target|pull_request_review|pull_request_review_comment)
        METADATA_ARGS+=("--build_metadata=RUN_TYPE=PULL_REQUEST")
        case "${GITHUB_REF:-}" in
          refs/pull/*)
            GH_PR="${GITHUB_REF#refs/pull/}"
            GH_PR="${GH_PR%%/*}"
            if [ -n "${GH_PR}" ]; then
              METADATA_ARGS+=("--build_metadata=PR_NUMBER=${GH_PR}")
              METADATA_ARGS+=("--build_metadata=PR_ID=${GH_PR}")
            fi
            ;;
        esac
        [ -n "${GITHUB_HEAD_REF:-}" ] && METADATA_ARGS+=("--build_metadata=PR_SOURCE_BRANCH_NAME=${GITHUB_HEAD_REF}")
        [ -n "${GITHUB_BASE_REF:-}" ] && METADATA_ARGS+=("--build_metadata=PR_TARGET_BRANCH_NAME=${GITHUB_BASE_REF}")
        ;;
      push)
        case "${GITHUB_REF:-}" in
          refs/tags/*) METADATA_ARGS+=("--build_metadata=RUN_TYPE=TAG") ;;
          *)           METADATA_ARGS+=("--build_metadata=RUN_TYPE=BRANCH_PUSH") ;;
        esac
        ;;
      schedule)                              METADATA_ARGS+=("--build_metadata=RUN_TYPE=SCHEDULED") ;;
      workflow_dispatch|repository_dispatch) METADATA_ARGS+=("--build_metadata=RUN_TYPE=MANUAL") ;;
    esac

  elif [ -n "${CIRCLECI:-}" ]; then
    METADATA_ARGS+=("--build_metadata=CI_HOST=CIRCLE_CI")
    [ -n "${CIRCLE_BRANCH:-}" ]    && METADATA_ARGS+=("--build_metadata=BRANCH_NAME=${CIRCLE_BRANCH}")
    [ -n "${CIRCLE_USERNAME:-}" ]  && METADATA_ARGS+=("--build_metadata=USER=${CIRCLE_USERNAME}")
    [ -n "${CIRCLE_BUILD_URL:-}" ] && METADATA_ARGS+=("--build_metadata=BUILD_URL=${CIRCLE_BUILD_URL}")
    if [ -n "${CIRCLE_PROJECT_USERNAME:-}" ] && [ -n "${CIRCLE_PROJECT_REPONAME:-}" ]; then
      METADATA_ARGS+=("--build_metadata=REPO_OWNER=${CIRCLE_PROJECT_USERNAME}")
      METADATA_ARGS+=("--build_metadata=REPO_NAME=${CIRCLE_PROJECT_REPONAME}")
      # CircleCI doesn't expose the VCS host directly — infer from the
      # repository URL's host (common case: github.com or bitbucket.org).
      case "${CIRCLE_REPOSITORY_URL:-}" in
        *github*)    METADATA_ARGS+=("--build_metadata=VCS=GITHUB")
                     METADATA_ARGS+=("--build_metadata=REPO_URL=https://github.com/${CIRCLE_PROJECT_USERNAME}/${CIRCLE_PROJECT_REPONAME}") ;;
        *bitbucket*) METADATA_ARGS+=("--build_metadata=VCS=BITBUCKET")
                     METADATA_ARGS+=("--build_metadata=REPO_URL=https://bitbucket.org/${CIRCLE_PROJECT_USERNAME}/${CIRCLE_PROJECT_REPONAME}") ;;
      esac
    fi
    if [ -n "${CIRCLE_PR_NUMBER:-}" ]; then
      METADATA_ARGS+=("--build_metadata=RUN_TYPE=PULL_REQUEST")
      METADATA_ARGS+=("--build_metadata=PR_NUMBER=${CIRCLE_PR_NUMBER}")
      METADATA_ARGS+=("--build_metadata=PR_ID=${CIRCLE_PR_NUMBER}")
      [ -n "${CIRCLE_BRANCH:-}" ] && METADATA_ARGS+=("--build_metadata=PR_SOURCE_BRANCH_NAME=${CIRCLE_BRANCH}")
    elif [ -n "${CIRCLE_TAG:-}" ]; then
      METADATA_ARGS+=("--build_metadata=RUN_TYPE=TAG")
      METADATA_ARGS+=("--build_metadata=TAG=${CIRCLE_TAG}")
    else
      METADATA_ARGS+=("--build_metadata=RUN_TYPE=BRANCH_PUSH")
    fi

  elif [ -n "${GITLAB_CI:-}" ]; then
    METADATA_ARGS+=("--build_metadata=CI_HOST=GITLAB")
    METADATA_ARGS+=("--build_metadata=VCS=GITLAB")
    [ -n "${CI_COMMIT_BRANCH:-}" ] && METADATA_ARGS+=("--build_metadata=BRANCH_NAME=${CI_COMMIT_BRANCH}")
    [ -n "${GITLAB_USER_NAME:-}" ] && METADATA_ARGS+=("--build_metadata=USER=${GITLAB_USER_NAME}")
    [ -n "${CI_JOB_URL:-}" ]       && METADATA_ARGS+=("--build_metadata=BUILD_URL=${CI_JOB_URL}")
    # Self-hosted GitLab — CI_SERVER_URL carries the host.
    if [ -n "${CI_PROJECT_NAMESPACE:-}" ] && [ -n "${CI_PROJECT_NAME:-}" ]; then
      GL_SERVER="${CI_SERVER_URL:-https://gitlab.com}"
      METADATA_ARGS+=("--build_metadata=REPO_OWNER=${CI_PROJECT_NAMESPACE}")
      METADATA_ARGS+=("--build_metadata=REPO_NAME=${CI_PROJECT_NAME}")
      METADATA_ARGS+=("--build_metadata=REPO_URL=${GL_SERVER}/${CI_PROJECT_NAMESPACE}/${CI_PROJECT_NAME}")
    fi
    # Merge request detection.
    if [ -n "${CI_MERGE_REQUEST_IID:-}" ] || [ "${CI_PIPELINE_SOURCE:-}" = "merge_request_event" ]; then
      METADATA_ARGS+=("--build_metadata=RUN_TYPE=PULL_REQUEST")
      [ -n "${CI_MERGE_REQUEST_IID:-}" ] && METADATA_ARGS+=("--build_metadata=PR_NUMBER=${CI_MERGE_REQUEST_IID}")
      [ -n "${CI_MERGE_REQUEST_IID:-}" ] && METADATA_ARGS+=("--build_metadata=PR_ID=${CI_MERGE_REQUEST_IID}")
      [ -n "${CI_MERGE_REQUEST_SOURCE_BRANCH_NAME:-}" ] && METADATA_ARGS+=("--build_metadata=PR_SOURCE_BRANCH_NAME=${CI_MERGE_REQUEST_SOURCE_BRANCH_NAME}")
      [ -n "${CI_MERGE_REQUEST_TARGET_BRANCH_NAME:-}" ] && METADATA_ARGS+=("--build_metadata=PR_TARGET_BRANCH_NAME=${CI_MERGE_REQUEST_TARGET_BRANCH_NAME}")
    elif [ -n "${CI_COMMIT_TAG:-}" ]; then
      METADATA_ARGS+=("--build_metadata=RUN_TYPE=TAG")
      METADATA_ARGS+=("--build_metadata=TAG=${CI_COMMIT_TAG}")
    elif [ "${CI_PIPELINE_SOURCE:-}" = "schedule" ]; then
      METADATA_ARGS+=("--build_metadata=RUN_TYPE=SCHEDULED")
    elif [ "${CI_PIPELINE_SOURCE:-}" = "web" ] || [ "${CI_PIPELINE_SOURCE:-}" = "api" ] || [ "${CI_PIPELINE_SOURCE:-}" = "trigger" ]; then
      METADATA_ARGS+=("--build_metadata=RUN_TYPE=MANUAL")
    else
      METADATA_ARGS+=("--build_metadata=RUN_TYPE=BRANCH_PUSH")
    fi
  fi
fi

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
  elif [ -n "${CIRCLE_PROJECT_REPONAME:-}" ]; then
    REPO_NAME=$(echo "${CIRCLE_PROJECT_REPONAME}" | sed 's|[^a-zA-Z0-9._-]|_|g')
  elif [ -n "${CI_PROJECT_NAME:-}" ]; then
    REPO_NAME=$(echo "${CI_PROJECT_NAME}" | sed 's|[^a-zA-Z0-9._-]|_|g')
  fi

  # Derive workspace subdir from the checkout path.
  # Mirrors the root_dir derivation in get_bazelrc_flags in environment.axl.
  WORKSPACE_DIR="${BUILDKITE_BUILD_CHECKOUT_PATH:-${GITHUB_WORKSPACE:-${CIRCLE_WORKING_DIRECTORY:-${CI_PROJECT_DIR:-$(pwd)}}}}"
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
export BAZEL_BUILD_OPTS="--config=ci ${BAZEL_REMOTE_FLAGS}"
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
  ${METADATA_ARGS[@]+"${METADATA_ARGS[@]}"} \
  -c dbg --remote_download_toplevel --show_progress_rate_limit=1 //:cli

# In GitHub Actions, env vars do not persist across steps unless written here.
if [ -n "${GITHUB_ENV:-}" ]; then
  {
    echo "BAZEL_STARTUP_OPTS=${BAZEL_STARTUP_OPTS}"
    echo "BAZEL_BUILD_OPTS=${BAZEL_BUILD_OPTS}"
    echo "DISABLE_PLUGINS_FLAG=${DISABLE_PLUGINS_FLAG}"
  } >> "${GITHUB_ENV}"
fi
