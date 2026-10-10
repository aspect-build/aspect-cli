#!/usr/bin/env python3
"""Exercise `BazelTrait.exec_log_event` against a real Bazel. Requires Bazel and the CLI.

Usage: python3 tools/exec-log-integration.py --aspect /path/to/aspect-cli
No remote cache, no credentials, no external Bazel dependencies: the fixture
workspace defines its own rules, so nothing is fetched. All fixtures and output
bases are temporary.

Why this exists, when `crates/aspect-cli/tests/exec_log_event.rs` already drives
every task that fires the hook: that harness uses the basil fake, whose log is
synthetic `file` entries — no spawns, no input sets, no runfiles trees. So the
resolver in `builtins/aspect/private/lib/execlog.axl` (input-set flattening,
runfiles trees, spawn keys, action fingerprints) and the `kinds` filter had only
ever been exercised against data shaped by the fake. This builds a workspace that
produces each of those entry kinds for real and asserts on what the hook received.
"""

import argparse
import json
import os
from pathlib import Path
import shutil
import subprocess
import tempfile
import time

# A CI image's `/etc/bazel.bazelrc` or a developer's `~/.bazelrc` can add a disk
# cache, a remote cache or a BES backend, any of which changes which actions
# execute and so what the execution log holds. Bazel rejects both flags in an rc
# file, so they go on the command line — as startup flags for `aspect`, which
# forwards them, and bare for a direct `bazel` call.
#
# Passing them is also how this job covers a bug it found: the CLI reads the
# Bazel server pid that nominates the execution log's holder, and Bazel kills a
# server whose startup options differ from the ones it is given. Read without
# these flags, that pid is the pid of a server the build replaces, and the log
# comes back silently empty.
STARTUP = ["--nosystem_rc", "--nohome_rc"]
ASPECT_STARTUP = [f"--bazel-startup-flag={f}" for f in STARTUP]

# A spawn whose primary output could not be resolved falls back to
# `<label>!<mnemonic>`, which collides with its siblings. Every spawn in a
# healthy log resolves, so the marker's absence is the assertion.
UNRESOLVED_KEY_MARKER = "!"

# The fixture's own rules: one action per rule, no external repositories. A
# depset with a transitive parent is what makes input-set flattening more than a
# single-level lookup, and a test rule's runfiles are what put a `runfiles_tree`
# entry in the log at all.
DEFS_BZL = '''
def _copy(ctx):
    out = ctx.actions.declare_file(ctx.label.name + ".copied")
    ctx.actions.run_shell(
        inputs = [ctx.file.src],
        outputs = [out],
        command = 'cat "$1" > "$2"',
        arguments = [ctx.file.src.path, out.path],
        mnemonic = "FixtureCopy",
    )
    return [DefaultInfo(files = depset([out]))]

copy = rule(
    implementation = _copy,
    attrs = {"src": attr.label(allow_single_file = True)},
)

def _combine(ctx):
    out = ctx.actions.declare_file(ctx.label.name + ".combined")

    # depset(direct, transitive = ...) is the shape the resolver has to walk:
    # the spawn names one input set id, which names another.
    inputs = depset(
        ctx.files.extra,
        transitive = [dep[DefaultInfo].files for dep in ctx.attr.deps],
    )
    ctx.actions.run_shell(
        inputs = inputs,
        outputs = [out],
        command = 'cat "$@" > ' + out.path,
        arguments = [f.path for f in inputs.to_list()],
        mnemonic = "FixtureCombine",
    )
    return [DefaultInfo(files = depset([out]))]

combine = rule(
    implementation = _combine,
    attrs = {
        "deps": attr.label_list(),
        "extra": attr.label_list(allow_files = True),
    },
)

def _fixture_test(ctx):
    out = ctx.actions.declare_file(ctx.label.name + ".sh")
    ctx.actions.write(out, "#!/bin/sh\\nexit 0\\n", is_executable = True)
    return [DefaultInfo(
        executable = out,
        runfiles = ctx.runfiles(files = ctx.files.data),
    )]

fixture_test = rule(
    implementation = _fixture_test,
    test = True,
    attrs = {"data": attr.label_list(allow_files = True)},
)
'''

BUILD = '''
load(":defs.bzl", "combine", "copy", "fixture_test")

copy(name = "first", src = "first.in")
copy(name = "second", src = "second.in")
combine(name = "joined", deps = [":first", ":second"], extra = ["extra.in"])
fixture_test(name = "runs", data = [":joined"])
'''

# One `exec_log_event` hook that resolves every entry it is handed, and a second
# that asked for spawns alone. Two hooks with different `kinds` is the shape that
# makes the runtime deliver their union and the AXL dispatcher re-check each
# entry's kind — the mixed path, which only a multi-kind log exercises.
CONFIG_AXL = '''"""Integration fixture: record what `exec_log_event` sees from a real build."""

load("@aspect//private/lib/execlog.axl", "SPAWN", "execlog")
load("@aspect//traits.axl", "BazelTrait", "ExecLogHook")

_REPORT_DIR = "{report_dir}"

# Bazel writes this itself: with no `--execution_log_compact_file` already on the
# command line, the CLI lends a `compact_file` sink's path to Bazel rather than
# minting a temp one, so the file is Bazel's own output byte for byte. It is what
# lets the assertions tell "Bazel logged nothing" from "we read nothing".
_BAZEL_LOG = "{bazel_log}"

# Per-spawn input lists are only there to be read by a failing assertion; the
# count beside them is what the assertions use.
_MAX_INPUTS = 64

def config(ctx: ConfigContext):
    res = execlog.resolver()
    kinds = {{}}
    spawns = []
    spawn_only = [0]
    ids = {{"zero": 0, "numbered": 0, "highest": 0, "out_of_order": 0}}

    def _resolve(_ctx, entry):
        res.observe(entry)
        kind = type(entry.type)
        kinds[kind] = kinds.get(kind, 0) + 1

        if entry.id == 0:
            ids["zero"] += 1
        else:
            ids["numbered"] += 1
            if entry.id <= ids["highest"]:
                ids["out_of_order"] += 1
            else:
                ids["highest"] = entry.id

        if kind != SPAWN:
            return

        fp = res.fingerprint(entry.type)
        inputs = res.inputs(entry.type)
        spawns.append({{
            "id": entry.id,
            "key": fp.key,
            "label": fp.label,
            "mnemonic": fp.mnemonic,
            "runner": fp.runner,
            "cache_hit": fp.cache_hit,
            "args_hash": fp.args_hash,
            "env_hash": fp.env_hash,
            "inputs_hash": fp.inputs_hash,
            "tools_hash": fp.tools_hash,
            "overall": fp.overall,
            "input_count": len(inputs),
            "inputs": inputs[:_MAX_INPUTS],
            "outputs": res.outputs(entry.type),
        }})

    def _count_spawns(_ctx, _entry):
        spawn_only[0] += 1

    def _report(c: TaskContext, exit_code: int):
        doc = {{
            "exit_code": exit_code,
            "observed": res.entries(),
            "kinds": kinds,
            "ids": ids,
            "spawn_only_count": spawn_only[0],
            "spawns": spawns,
        }}
        path = _REPORT_DIR + "/" + c.task.name + ".json"
        c.std.fs.create(path).write(json.encode(doc, indent = 2) + "\\n")

    t = ctx.traits[BazelTrait]
    t.execution_log_sinks.append(bazel.execution_log.compact_file(path = _BAZEL_LOG))
    t.exec_log_event.append(ExecLogHook(on_entry = _resolve, kinds = execlog.RESOLVER_KINDS))
    t.exec_log_event.append(ExecLogHook(on_entry = _count_spawns, kinds = [SPAWN]))
    t.build_end.append(_report)
'''


def scaffold(root: Path, report_dir: Path, bazel_version: str, output_base: Path):
    """Write the fixture workspace.

    The JVM heap is left at Bazel's own default: a cap tuned for another fixture
    is a way to have the server die and be restarted mid-build, which this test
    would then read as an empty log. `--nosystem_rc` / `--nohome_rc` cannot be
    set here — Bazel rejects them in an rc file — so they go on each command
    line; see `STARTUP`.
    """
    (root / "MODULE.bazel").write_text('module(name = "execlog_fixture")\n')
    (root / "MODULE.aspect").write_text("")
    (root / ".bazelversion").write_text(bazel_version + "\n")
    (root / "defs.bzl").write_text(DEFS_BZL)
    (root / "BUILD").write_text(BUILD)
    for name in ("first.in", "second.in", "extra.in"):
        (root / name).write_text(name + " v1\n")
    (root / ".aspect").mkdir()
    (root / ".aspect/config.axl").write_text(
        CONFIG_AXL.format(report_dir=report_dir, bazel_log=root / "bazel-wrote-this.zst")
    )
    (root / ".bazelrc").write_text(
        f"startup --output_base={output_base}\n"
        "common --lockfile_mode=off\n"
        "build --bes_backend=\n"
        "build --spawn_strategy=local\n"
    )


# A compact log holding no entries is still a valid zstd frame, and Bazel writes
# one for a build that ran no actions. Measured at 66 bytes; the bound is
# deliberately loose because it is only used to tell an empty log from a full one.
EMPTY_LOG_CEILING = 256


def report(report_dir: Path, workspace: Path, task_name: str) -> dict:
    """The hooks' record for `task_name`, with Bazel's own log size beside it.

    `bazel_log_bytes` is what makes a zero-entry failure name its own cause: the
    hooks and that file read the same bytes from the same path, so a full log
    beside an empty hook is the CLI's reader losing entries, and an empty one is
    Bazel never having written them.
    """
    path = report_dir / f"{task_name}.json"
    log = workspace / "bazel-wrote-this.zst"
    assert path.exists(), f"the exec_log_event hooks never reported to {path}"
    doc = json.loads(path.read_text())
    doc["bazel_log_bytes"] = log.stat().st_size if log.exists() else None
    path.unlink()
    return doc


def spawns_by_mnemonic(doc: dict, mnemonic: str) -> list:
    return [s for s in doc["spawns"] if s["mnemonic"] == mnemonic]


def check_shared(doc: dict, label: str):
    """The invariants that hold for any real log, whatever produced it."""
    wrote = doc["bazel_log_bytes"]
    if doc["observed"] == 0:
        # Zero entries is the failure this whole job exists to catch, so say which
        # side lost them rather than leaving the next reader to bisect a matrix.
        if wrote is None:
            raise AssertionError(
                f"{label}: the hook received no entries, and bazel wrote no "
                f"execution log either. Bazel creates the file as soon as it starts "
                f"a build, so this is an invocation that never got that far — or one "
                f"that was never given the flag."
            )
        if wrote > EMPTY_LOG_CEILING:
            raise AssertionError(
                f"{label}: bazel wrote a {wrote}-byte execution log and the hook "
                f"received none of it. The CLI's reader lost this log — not a Bazel "
                f"difference. Check the WARNING line in the build output."
            )
        raise AssertionError(
            f"{label}: the hook received no entries, and bazel's own log is "
            f"{wrote} bytes — an empty frame, so bazel logged no actions for this "
            f"build. Something made every action a cache hit or skipped execution."
        )
    assert wrote is None or wrote > EMPTY_LOG_CEILING, (
        f"{label}: the hook received {doc['observed']} entries but bazel's own log "
        f"is only {wrote} bytes; the two read the same path and must agree"
    )
    assert doc["ids"]["out_of_order"] == 0, (
        f"{label}: entry ids must advance; a repeat or a reorder means the reader "
        f"delivered two builds' logs, or one of them twice: {doc['ids']}"
    )
    assert doc["kinds"].get("file", 0) > 0, f"{label}: no file entries: {doc['kinds']}"
    assert doc["kinds"].get("input_set", 0) > 0, (
        f"{label}: no input_set entries, so nothing exercised input-set "
        f"flattening: {doc['kinds']}"
    )

    # Two hooks asked for different kinds, so the runtime delivered their union
    # and the AXL dispatcher re-checked every entry. The narrow hook must have
    # been handed the spawns and nothing else.
    assert doc["spawn_only_count"] == doc["kinds"].get("spawn", 0), (
        f"{label}: a kinds=[\"spawn\"] hook received {doc['spawn_only_count']} "
        f"entries beside a resolving hook's {doc['kinds'].get('spawn', 0)} spawns"
    )

    # Bazel numbers only the entries something refers to by id, and nothing
    # refers to a spawn. Documented on `ExecLogHook`; asserted here because the
    # fake cannot show it.
    assert doc["ids"]["zero"] == doc["kinds"].get("spawn", 0), (
        f"{label}: every spawn should arrive with id = 0 and nothing else should: "
        f"{doc['ids']}, spawns={doc['kinds'].get('spawn', 0)}"
    )

    for spawn in doc["spawns"]:
        assert UNRESOLVED_KEY_MARKER not in spawn["key"], (
            f"{label}: {spawn['label']} fell back to the label key, so its primary "
            f"output id did not resolve: {spawn['key']}"
        )
        assert spawn["outputs"], f"{label}: {spawn['key']} resolved no outputs"
        assert len(spawn["overall"]) == 64, (
            f"{label}: {spawn['key']} has no action fingerprint: {spawn['overall']}"
        )


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--aspect", required=True)
    parser.add_argument("--bazel", default="bazel")
    parser.add_argument("--bazel-version", default="9.2.0")
    args = parser.parse_args()
    aspect = str(Path(shutil.which(args.aspect) or args.aspect).resolve())
    bazel = str(Path(shutil.which(args.bazel) or args.bazel).resolve())
    started = time.monotonic()

    with tempfile.TemporaryDirectory(prefix="exec-log-integration-") as tmp:
        root = Path(tmp)
        workspace = root / "workspace"
        workspace.mkdir()
        reports = root / "reports"
        reports.mkdir()
        output_base = root / "output"
        scaffold(workspace, reports, args.bazel_version, output_base)

        # A developer's or runner's own ASPECT_*/CI environment would add status
        # surfaces, deployments and auth to a test about entry resolution.
        env = {
            k: v
            for k, v in os.environ.items()
            if k != "CI"
            and not k.startswith(
                ("ASPECT_", "BAZEL_", "BUILDKITE_", "CIRCLE", "GITHUB_", "GITLAB_")
            )
        }
        env["BAZEL_REAL"] = bazel
        env["USE_BAZEL_VERSION"] = args.bazel_version
        timings = {}
        # Kept so an assertion about what the hooks saw can show the build that
        # produced it; a `check_shared` failure otherwise says nothing about the
        # invocation, which is most of the evidence.
        last_output = {}

        def run(name, command, expected=0):
            before = time.monotonic()
            result = subprocess.run(
                command, cwd=workspace, env=env, text=True, capture_output=True, timeout=600
            )
            timings[name] = round(time.monotonic() - before, 3)
            last_output[name] = result.stdout + result.stderr
            last_output["latest"] = f"--- {name} ---\n" + last_output[name]
            print(f"{name}: exit={result.returncode}, seconds={timings[name]}", flush=True)
            assert result.returncode == expected, result.stdout + result.stderr
            return result

        try:
            version = run("bazel_version", [bazel, "--version"])
            # Recorded, not just asserted: a run reported against the wrong
            # version is a verification that did not happen.
            print(
                f"requested={args.bazel_version} resolved={version.stdout.strip()!r} "
                f"bazel={bazel}",
                flush=True,
            )
            assert version.stdout.strip() == f"bazel {args.bazel_version}", (
                version.stdout + version.stderr
            )

            # 1. A cold build: every action runs, so every action is logged.
            run("build", [aspect, "build", "--task:name=cold-build", *ASPECT_STARTUP, "--", "//..."])
            cold = report(reports, workspace, "cold-build")
            check_shared(cold, "cold build")

            combine = spawns_by_mnemonic(cold, "FixtureCombine")
            assert len(combine) == 1, f"expected one FixtureCombine spawn: {cold['spawns']}"
            inputs = combine[0]["inputs"]
            assert combine[0]["input_count"] == 3, (
                "the combine action's input set nests another set and adds a source "
                f"file; flattening it must yield all three: {inputs}"
            )
            assert any("first.copied" in i for i in inputs), inputs
            assert any("second.copied" in i for i in inputs), inputs
            assert any("extra.in" in i for i in inputs), inputs
            for item in inputs:
                path, _, digest = item.rpartition("@")
                assert digest, f"a flattened input carries no digest: {item}"

            copies = spawns_by_mnemonic(cold, "FixtureCopy")
            assert len(copies) == 2, f"expected two FixtureCopy spawns: {cold['spawns']}"
            assert {c["inputs_hash"] for c in copies}.__len__() == 2, (
                "two copies of different sources must hash to different inputs"
            )

            # 2. A test run: a test's runfiles are the only thing that puts a
            #    runfiles_tree entry in the log, and a spawn names the tree
            #    rather than the set inside it — so the resolver has to treat the
            #    tree as a set or every runfile drops out of the action key.
            run("test", [aspect, "test", "--task:name=test-run", *ASPECT_STARTUP, "--", "//..."])
            tested = report(reports, workspace, "test-run")
            check_shared(tested, "test run")
            assert tested["kinds"].get("runfiles_tree", 0) > 0, (
                f"the test run logged no runfiles tree: {tested['kinds']}"
            )
            runners = [s for s in tested["spawns"] if "runs" in s["key"] or s["mnemonic"] == "TestRunner"]
            assert runners, f"no test spawn in the log: {tested['spawns']}"
            assert any(
                any("joined.combined" in i for i in s["inputs"]) for s in runners
            ), (
                "a test spawn's inputs must include what its runfiles tree holds: "
                f"{[s['inputs'] for s in runners]}"
            )

            # 3. Change one source. The action that reads it re-runs, and its
            #    inputs fingerprint must move with the digest it depends on —
            #    the resolver's whole purpose.
            (workspace / "first.in").write_text("first.in v2\n")
            run("rebuild", [aspect, "build", "--task:name=warm-build", *ASPECT_STARTUP, "--", "//..."])
            warm = report(reports, workspace, "warm-build")
            check_shared(warm, "rebuild")
            rebuilt = {s["key"]: s for s in warm["spawns"]}
            before = {s["key"]: s for s in cold["spawns"]}
            changed = [k for k in rebuilt if k in before]
            assert changed, (
                "the rebuild re-ran no action the cold build had logged, so there is "
                f"nothing to compare: {list(rebuilt)} vs {list(before)}"
            )
            moved = [k for k in changed if rebuilt[k]["inputs_hash"] != before[k]["inputs_hash"]]
            assert moved, (
                "an action re-ran after its input changed, but every inputs "
                "fingerprint is identical — the resolver is not reading digests"
            )

            print(
                json.dumps(
                    {
                        "timings": timings,
                        "total_seconds": round(time.monotonic() - started, 3),
                        "cold_entries": cold["observed"],
                        "cold_kinds": cold["kinds"],
                        "test_kinds": tested["kinds"],
                        "inputs_fingerprints_moved": moved,
                    },
                    indent=2,
                ),
                flush=True,
            )
        except AssertionError:
            print(last_output.get("latest", "(no build ran)"), flush=True)
            raise
        finally:
            subprocess.run(
                [bazel, *STARTUP, f"--output_base={output_base}", "shutdown"],
                cwd=workspace,
                env=env,
                capture_output=True,
                timeout=60,
            )


if __name__ == "__main__":
    main()
