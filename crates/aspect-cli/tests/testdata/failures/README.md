# BEP failure fixtures

Recorded Bazel output for every failure shape the Failures summary has to handle, on Bazel
**7.7.3, 8.9.0 and 9.3.3**. `aspect dev test-failure-replay` feeds each recording through the
failure summary and the Dashboard's Failures tab and compares the result with `<ver>.golden`
(`--update` rewrites them; `--case=<name>` replays one case). The replay needs no Bazel.

## Refreshing

Recording needs Bazelisk (`$BAZELISK`, else `bazelisk` or `bazel` on PATH), bash, perl,
python3, `script` (BSD or util-linux) and network access for Bazel and its modules. Bazelisk's
cache is `$BAZELISK_HOME` if set, else its own default. Bazel runs under `env -i` with a
throwaway HOME, so no rc file, environment variable or token of yours reaches a recording;
`tools/failure-fixtures/env.sh` says exactly what it gets. Scratch state goes under
`$FIXTURE_SCRATCH` (default `${TMPDIR:-/tmp}/aspect-failure-fixtures`).

```sh
# One case on one version, then its goldens:
tools/failure-fixtures/record.sh xn 9.3.3
aspect dev test-failure-replay --update --case=xn

# Every case on every recorded version:
tools/failure-fixtures/refresh_all.sh 7.7.3 8.9.0 9.3.3
aspect dev test-failure-replay --update
```

Review the golden diff: event order and timings vary run to run (see Non-determinism), so a
re-recorded case can reorder its entries without anything having changed. A new case is a
folder with `src/` (Bazel files suffixed `.in`), `cmd` and `variants`; record it, then
`--update`.

## Layout

```
<case>/
  src/                      standalone workspace (MODULE.bazel + packages); builds on its own once
                            the `.in` suffix its Bazel files carry (so this repository's Bazel
                            and formatters leave them alone) is dropped, as record.sh does
  cmd                       the bazel arguments, exactly (startup options first, as for `bazel`)
  variants                  extra recordings: noall, nocolor, merged, console_only
  <ver>.bep.json            --build_event_json_file output (cmd includes --build_event_publish_all_actions)
  <ver>.console             Bazel's stderr, written to a pty, --color=yes --curses=yes
  <ver>.stdout              Bazel's stdout, a file of its own, only when non-empty: Bazel relays an
                            action's stdout here, so it is not in the console (as wherever the
                            CLI's stdout is not its stderr)
  <ver>.exit                Bazel's exit code ("timeout" if the 90s console_only cap hit)
  <ver>.files/<path>        copies of the action stderr/stdout and test.log/test.xml files the BEP names
  <ver>.noall.*             same command without --build_event_publish_all_actions
  <ver>.nocolor.*           same command with --color=no --curses=no
  <ver>.merged.*            same command with Bazel's stdout on the pty too, as a terminal shares it
                            with stderr: an action's stdout is in the console, after its stderr
  <ver>[.<variant>].golden  the replay test's expected output for that recording
```

The recorder drops the BEP events the summary never reads, which were most of the bytes:
`progress` (Bazel's console chunks), `structuredCommandLine`, `unstructuredCommandLine` and
`buildToolLogs`. An Aborted one of those is kept (a bad flag's `unstructuredCommandLine`).

The recorder puts `--nosystem_rc` before `cmd`, with the startup options a `cmd` starts with
(`oom`'s `--host_jvm_args`), and adds these after it: `--disk_cache=`, `--lockfile_mode=off`,
the colour flags, `--build_event_json_file` and an `--action_env` of its own, so each
recording runs its actions again rather than taking them from Bazel's action cache (a
successful action's `INFO: From` output is in every variant). Bazel runs under
`env -i` with an empty HOME, so no user rc file or environment variable reaches the recording.
Each case is preceded by an unrecorded `--nobuild` run of the same command so module
resolution and repository fetches do not show in whichever case happens to run first; the
recorded run therefore sees a warm analysis cache, as an incremental build would.

## Placeholders

Machine-specific strings in `*.bep.json`, `*.console`, `*.stdout` and `*.files/**` are replaced:

| Placeholder | Was |
|---|---|
| `${OUTPUT_BASE}` | `bazel info output_base` |
| `${INSTALL_BASE}` | `bazel info install_base` |
| `${OUTPUT_USER_ROOT}` | parent of the output base |
| `${WORKSPACE}` | the scratch workspace, in its `/private/tmp`, `/tmp` and realpath spellings |
| `${RAW}` | the recorder's temporary directory (appears in `--build_event_json_file=`) |
| `${HOME}` | the throwaway HOME and the real one |
| `${DARWIN_USER_DIR}` | macOS `/var/folders/<x>/<y>` |
| `${USER}`, `${HOST}` | `BuildStarted.user`, `BUILD_USER`, `BUILD_HOST`, `USER=`/`LOGNAME=` env values, short hostname |

A BEP file URI `file://${OUTPUT_BASE}/<p>` resolves to `<ver>.files/<p>` in the same fixture
(e.g. `execroot/_main/bazel-out/_tmp/actions/stderr-3`, `.../testlogs/t/fail/test.log`).

## Non-determinism

- Event order and timings vary run to run (parallel actions); assert on sets, not order.
- `t`: `//t:flaky` fails attempt 1 and passes attempt 2; exactly one of `//t:rpt`'s three runs
  fails, but which one varies. Both use marker paths under `/tmp/bepfx-*`, cleared per run.
- `oom` is console only: Bazel 9 crashes with `FATAL: bazel ran out of memory`; older versions
  may instead thrash until the 90s cap.
