#!/usr/bin/env bash

set -eu -o pipefail

owner="aspect-build"
repo="aspect-cli"
list_asset_url="https://api.github.com/repos/${owner}/${repo}/releases/latest"

echo >&2 "Attempting to identify the host os ..."
case "$(uname -o)" in
    Darwin)
        os=macos
        ;;
    Linux)
        os=linux
        ;;
    *)
        echo >&2 "Error: Unable to pick the right os! Correct the first uname match"
        exit 1
        ;;
esac

echo >&2 "Attempting to identify the host arch ..."
case "$(uname -m)" in
    arm64|aarch64)
        arch=aarch64
        ;;
    amd64|x86_64)
        arch=x86_64
        ;;
    *)
        echo >&2 "Error: Unable to pick the right arch! Correct the second uname match"
        exit 1
        ;;
esac

artifact="aspect-launcher-${os}_${arch}"

echo >&2 "Attempting to identify the most recent release asset ..."
asset_url=$(curl -s "${list_asset_url}" | jq ".assets[] | select(.name==\"${artifact}\") | .url" | sed 's/\"//g' | head -n 1)

echo >&2 "Attempting to fetch the release asset ${asset_url} ..."
curl -sLJ \
    -H 'Accept: application/octet-stream' \
    -o aspect \
     "${asset_url}"

chmod +x ./aspect

./aspect help

echo "Success! Move the aspect (launcher) binary onto your \$PATH"
