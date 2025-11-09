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

  # Find GitHub release of the aspect-launcher
  release="$version"
  if [ "$release" != "latest" ]; then
    release="tags/$release"
  fi
  url=$(
    curl -fsSL "https://api.github.com/repos/aspect-build/aspect-cli/releases/$release" |
      perl -nle 'if (/"browser_download_url":\s*"(.*aspect-launcher-'"${arch}-${platform}"')"/) { print $1 }'
  ) || true
  if [[ ! "$url" ]]; then
    echo >&2 "[ERROR] could not find an aspect launcher release '$version' for OS '$os' ($platform), arch '$arch'"
    exit 1
  fi

  # Download and install to /usr/local/bin
  echo >&2 "downloading $url"
  curl -fSL "$url" -o "$tmp"
  chmod 0755 "$tmp"
  echo >&2 "installing aspect launcher to /usr/local/bin/aspect (system may prompt for password)"
  sudo mv "$tmp" /usr/local/bin/aspect
)

install_aspect_launcher "$@"
