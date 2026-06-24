#!/usr/bin/env bash
set -o errexit -o nounset -o pipefail

install_aspect_launcher() (
    # Get host arch ("x86_64" or "aarch64") & os ("linux" or "darwin")
    arch=$(uname -m)
    if [ "$arch" = "arm64" ]; then arch="aarch64"; fi
    os=$(uname -s | tr '[:upper:]' '[:lower:]')

    # Map to GitHub asset platform suffix
    if [ "$os" = "darwin" ]; then
        platform="apple-darwin"
    elif [ "$os" = "linux" ]; then
        platform="unknown-linux-musl"
    else
        echo >&2 "[ERROR] unsupported os: $os"
        exit 1
    fi

    # Create tmp directory to download to
    tmp=$(mktemp aspect-launcher.XXXXX)
    cleanup() { rm -f "$tmp"; }
    trap cleanup EXIT

    # Determine which version to install
    version="${1:-latest}"
    if [[ $version =~ ^[0-9]{4}\.[0-9]+\.[0-9]+$ ]]; then
        version="v$version"
    fi
    echo >&2 "installing aspect launcher $version"

    # Asset name is deterministic given (arch, platform), so when the caller
    # pinned a concrete version we can skip the GitHub API lookup and download
    # the release asset directly. That dodges api.github.com's anonymous
    # 60/hr-per-IP rate limit, which is what CI runners on shared egress IPs
    # routinely trip into (curl 22 / 403).
    asset="aspect-launcher-${arch}-${platform}"
    if [ "$version" = "latest" ]; then
        # Have to ask the API which release `latest` points at. Honor
        # GITHUB_TOKEN if set so authenticated callers get the 5000/hr budget.
        #
        # auth_args is expanded as ${auth_args[@]+"..."} rather than the bare
        # "${auth_args[@]}" because expanding an empty array under `set -u` is an
        # unbound-variable error in Bash 3.2 — which is what macOS ships — and the
        # token-less `curl | bash` path leaves the array empty.
        auth_args=()
        if [ -n "${GITHUB_TOKEN:-}" ]; then
            auth_args=(-H "Authorization: Bearer $GITHUB_TOKEN")
        fi
        api="https://api.github.com/repos/aspect-build/aspect-cli/releases/latest"
        if ! release_json=$(curl -fsSL ${auth_args[@]+"${auth_args[@]}"} "$api"); then
            echo >&2 "[ERROR] failed to query GitHub for the latest release: $api"
            echo >&2 "[ERROR] (anonymous api.github.com is rate-limited to 60/hr per IP; set GITHUB_TOKEN to raise the limit, or pin a concrete version to skip the API)"
            exit 1
        fi
        url=$(printf '%s' "$release_json" |
            perl -nle 'if (/"browser_download_url":\s*"(.*'"$asset"')"/) { print $1 }')
        if [[ ! "$url" ]]; then
            echo >&2 "[ERROR] could not find an aspect launcher release '$version' for OS '$os' ($platform), arch '$arch'"
            exit 1
        fi
    else
        # Deterministic URL — no API lookup required.
        url="https://github.com/aspect-build/aspect-cli/releases/download/${version}/${asset}"
    fi

    # Download and install to /usr/local/bin
    echo >&2 "downloading $url"
    curl -fSL "$url" -o "$tmp"
    chmod 0755 "$tmp"
    echo >&2 "installing aspect launcher to /usr/local/bin/aspect (system may prompt for password)"
    sudo mv "$tmp" /usr/local/bin/aspect
)

install_aspect_launcher "$@"
