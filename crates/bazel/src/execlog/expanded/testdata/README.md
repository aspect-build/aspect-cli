# Execution log fixtures

`v<version>/` holds the three execution logs Bazel wrote for one build of
`workspace/`: a genrule chain, a tree artifact, and a tool with runfiles. The
tests convert `compact.zstd` and compare the result with `binary.binpb` and
`log.json`, leaving out the timings, which change from run to run. A strict
action environment keeps the machine's `PATH` out of them.

The workspace's files end in `.in` so this repository's Bazel doesn't load them
as a package. To regenerate a version's logs, set `VERSION`, and `FLAGS` to
`--legacy_external_runfiles --experimental_sibling_repository_layout` for
Bazel 7 and 8 (later versions dropped both), then run:

```sh
ws=$(mktemp -d)
for f in workspace/*.in; do n=${f##*/}; cp "$f" "$ws/${n%.in}"; done
out=$PWD/v$VERSION && mkdir -p "$out" && cd "$ws"
for flag in compact_file="$out/compact.zstd" binary_file="$out/binary.binpb" json_file="$out/log.json"; do
  USE_BAZEL_VERSION=$VERSION bazel --ignore_all_rc_files clean
  USE_BAZEL_VERSION=$VERSION bazel --ignore_all_rc_files build //:u //:a \
    --lockfile_mode=off --disk_cache= --noremote_accept_cached \
    --incompatible_strict_action_env \
    $FLAGS "--execution_log_$flag"
done
```
