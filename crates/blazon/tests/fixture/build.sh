#!/bin/sh
# Rebuilds fixture.jar from src/. Needs a JDK (`javac`, `jar`); the jar is checked in.
set -eu
here=$(cd "$(dirname "$0")" && pwd)
out=$(mktemp -d)
javac --release 17 -parameters -d "$out" $(find "$here/src" -name '*.java')
jar --create --file "$here/fixture.jar" -C "$out" .
rm -rf "$out"
