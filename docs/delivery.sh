#!/usr/bin/env bash

set -o errexit -o nounset -o pipefail

msg="Demostration delivery target"
echo $msg

export FOO="${BAR}"
