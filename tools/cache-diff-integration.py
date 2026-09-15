#!/usr/bin/env python3
"""Exercise cache diff against an isolated REAPI cache. Requires Bazel and the CLI.

Usage: python3 tools/cache-diff-integration.py --aspect /path/to/aspect-cli
The pinned cache binary is downloaded and checksum-verified; no Docker or cloud
credentials are needed. All fixtures, cache entries and servers are temporary.
"""

import argparse
from contextlib import ExitStack
import hashlib
import json
import os
from pathlib import Path
import platform
import shutil
import shlex
import socket
import subprocess
import tempfile
import time
import urllib.request

CACHE_VERSION = "2.6.2"
CACHE_SHA256 = {
    "darwin-amd64": "fc3b8f5d8edcf94f578cf6ffa8071d8859f5f02a38c14ae00cd3d22aa44974b6",
    "darwin-arm64": "1d6bc5fe5be7a66c7aeea6723280402414e280ae579016d3b5e6e5c77278ad04",
    "linux-amd64": "62e236bf8396e69396928e0d0c32062fbd5575f20fe55dc10a82eb791297e1a0",
    "linux-arm64": "b2cabd5bf674e8d0649de2c95dddfea8d875db1a06ed9d431674c9293df273ed",
}


def free_port():
    with socket.socket() as sock:
        sock.bind(("127.0.0.1", 0))
        return sock.getsockname()[1]


def download_cache(binary, target):
    if target not in CACHE_SHA256:
        raise RuntimeError(f"unsupported cache integration platform: {target}")
    url = f"https://github.com/buchgr/bazel-remote/releases/download/v{CACHE_VERSION}/bazel-remote-{CACHE_VERSION}-{target}"
    for attempt in range(3):
        try:
            # urlopen uses the standard HTTP(S)_PROXY / NO_PROXY environment.
            with urllib.request.urlopen(url, timeout=60) as response:
                content = response.read()
            break
        except OSError:
            if attempt == 2:
                raise
            time.sleep(attempt + 1)
    if hashlib.sha256(content).hexdigest() != CACHE_SHA256[target]:
        raise RuntimeError("cache binary checksum mismatch")
    binary.write_bytes(content)
    binary.chmod(0o700)


def stop_cache(server):
    server.terminate()
    try:
        server.wait(timeout=10)
    except subprocess.TimeoutExpired:
        server.kill()
        server.wait()


def start_cache(binary, root, env, log):
    for attempt in range(3):
        http_port, grpc_port = free_port(), free_port()
        while grpc_port == http_port:
            grpc_port = free_port()
        log.seek(0)
        log.truncate()
        server = subprocess.Popen([str(binary), "--dir", str(root / "cache"), "--max_size", "1",
                                   "--http_address", f"127.0.0.1:{http_port}",
                                   "--grpc_address", f"127.0.0.1:{grpc_port}"], env=env, stdout=log, stderr=subprocess.STDOUT)
        for _ in range(100):
            if server.poll() is not None:
                break
            try:
                with urllib.request.urlopen(f"http://127.0.0.1:{http_port}/status", timeout=1):
                    return server, http_port, grpc_port
            except OSError:
                time.sleep(0.1)
        stop_cache(server)
        log.seek(0)
        message = log.read()
        # Port reservation cannot be handed to bazel-remote. Retry a bind race
        # with fresh ports; holding sockets through Popen would prevent its bind.
        if "address already in use" not in message.lower() or attempt == 2:
            raise RuntimeError("cache server did not become ready: " + message)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--aspect", required=True)
    parser.add_argument("--bazel", default="bazel")
    parser.add_argument("--bazel-version", default="9.2.0")
    args = parser.parse_args()
    aspect = str(Path(shutil.which(args.aspect) or args.aspect).resolve())
    bazel = str(Path(shutil.which(args.bazel) or args.bazel).resolve())
    started = time.monotonic()
    with tempfile.TemporaryDirectory(prefix="cache-diff-integration-") as tmp, ExitStack() as cleanup:
        root = Path(tmp)
        arch = {"x86_64": "amd64", "aarch64": "arm64"}.get(platform.machine(), platform.machine())
        target = f"{platform.system().lower()}-{arch}"
        binary = root / "bazel-remote"
        download_cache(binary, target)
        env = {k: v for k, v in os.environ.items() if k != "CI" and not k.startswith(("BAZEL_REMOTE_", "ASPECT_", "GITHUB_", "BUILDKITE_"))}
        env["BAZEL_REAL"] = bazel
        env["USE_BAZEL_VERSION"] = args.bazel_version
        log = cleanup.enter_context((root / "server.log").open("w+"))
        server, http_port, grpc_port = start_cache(binary, root, env, log)
        cleanup.callback(stop_cache, server)
        workspace = root / "workspace"
        workspace.mkdir()
        (workspace / ".bazelversion").write_text(args.bazel_version + "\n")
        (workspace / "MODULE.bazel").write_text('module(name = "cache_fixture")\n')
        (workspace / "MODULE.aspect").write_text("")
        (workspace / "shared.in").write_text("baseline\n")
        (workspace / "personal").mkdir()
        markers = root / "markers"
        markers.mkdir()
        (workspace / "defs.bzl").write_text('''
def _shared(ctx):
    out = ctx.actions.declare_file(ctx.label.name + ".out")
    ctx.actions.run_shell(inputs = [ctx.file.src], outputs = [out],
        command = BUILD_COMMAND,
        arguments = [ctx.file.src.path, out.path], mnemonic = "FixtureCopy")
    return [DefaultInfo(files = depset([out]))]
shared = rule(implementation = _shared, attrs = {"src": attr.label(allow_single_file = True)})
def _test(ctx):
    out = ctx.actions.declare_file(ctx.label.name + ".sh")
    ctx.actions.write(out, TEST_SCRIPT, is_executable = True)
    return [DefaultInfo(executable = out, runfiles = ctx.runfiles(files = ctx.files.data))]
fixture_test = rule(implementation = _test, test = True, attrs = {"data": attr.label_list(allow_files = True)})
'''.replace("BUILD_COMMAND", json.dumps(f'echo build >> {shlex.quote(str(markers / "build"))}; cat "$1" > "$2"')).replace(
            "TEST_SCRIPT", json.dumps(f"#!/bin/sh\necho test >> {shlex.quote(str(markers / 'test'))}\nexit 0\n")))
        (workspace / "BUILD").write_text('''
load(":defs.bzl", "fixture_test", "shared")
shared(name = "shared", src = "shared.in", visibility = ["//visibility:public"])
fixture_test(name = "a", data = [":shared"])
fixture_test(name = "b", data = [":shared"])
fixture_test(name = "noci", data = [":shared"], tags = ["noci"])
fixture_test(name = "unrelated")
test_suite(name = "suite", tests = [":a", ":b", ":noci"], tags = ["manual"])
''')
        (workspace / "personal/BUILD").write_text('''
load("//:defs.bzl", "fixture_test")
fixture_test(name = "excluded", data = ["//:shared"])
''')
        startup = [ f"--output_base={root / 'output'}", "--host_jvm_args=-Xmx512m"]
        flags = [f"--remote_cache=grpc://127.0.0.1:{grpc_port}", "--remote_upload_local_results",
                 "--jobs=2", "--local_test_jobs=2", "--remote_timeout=15", "--remote_retries=0",
                 f"--sandbox_writable_path={markers}", "--lockfile_mode=off", "--bes_backend=", "--test_env=FOO=1"]
        (workspace / ".bazelrc").write_text("".join(f"startup {f}\n" for f in startup) + "".join(f"build {f}\n" for f in flags))
        timings = {}

        def run(name, command, expected=0):
            before = time.monotonic()
            result = subprocess.run(command, cwd=workspace, env=env, text=True, capture_output=True, timeout=180)
            timings[name] = round(time.monotonic() - before, 3)
            print(f"{name}: exit={result.returncode}, seconds={timings[name]}", flush=True)
            assert result.returncode == expected, result.stdout + result.stderr
            return result

        def count(kind):
            path = markers / kind
            return len(path.read_text().splitlines()) if path.exists() else 0

        def probe(name, mode="overreport", patterns=None, expected=None, total=3, extra_flags=None):
            before_tests, before_builds = count("test"), count("build")
            result = run(name, [aspect, "cache", "diff", "--bazel-startup-flag=--nosystem_rc", "--bazel-startup-flag=--nohome_rc", f"--mode={mode}", "--output=json", "--test_tag_filters=-noci"] + (extra_flags or []) + ["--"] + (patterns or ["//...", "-//personal/..."]))
            if "--announce-bazel-command=true" in (extra_flags or []):
                assert "--noanalyze" in result.stderr, result.stderr
            doc = json.loads(result.stdout)
            got = {row["label"] for row in doc["affected"]}
            assert got == set(expected or []), (name, doc)
            assert doc["total_tests"] == total, (name, doc)
            assert count("test") == before_tests, f"{name}: probe executed a test"
            if mode == "overreport":
                assert count("build") == before_builds, f"{name}: probe executed a build action"
            return doc

        try:
            version = run("bazel_version", [bazel, "--version"])
            assert version.stdout.strip() == f"bazel {args.bazel_version}", version.stdout + version.stderr
            run("seed", [bazel, "--nosystem_rc", "--nohome_rc", "test", "//..."])
            assert count("test") == 5, "baseline did not execute every fixture test"
            assert count("build") == 1, "baseline did not build the shared dependency"
            probe("warm_hits")
            with (workspace / ".bazelrc").open("a") as rc:
                rc.write("build:selection_only --test_env=FOO=1\n")
            probe("build_only_config_warm_hits", extra_flags=["--config=selection_only"])
            probe("announced_selection", extra_flags=["--announce-bazel-command=true"])
            (workspace / "shared.in").write_text("changed\n")
            doc = probe("dependency_miss", expected=["//:a", "//:b"])
            assert all(any(c["target"] == "//:shared" for c in row["caused_by"]) for row in doc["affected"]), doc
            probe("command_specific_flags_miss", expected=["//:a", "//:b"], extra_flags=[
                "--test_env", "FOO=1", "--runs_per_test=1", "--build_tests_only",
                "--remote_download_outputs=minimal", f"--remote_cache=grpc://127.0.0.1:{grpc_port}",
            ])
            probe("suite_miss", patterns=["//:suite", "//:unrelated"], expected=["//:a", "//:b"])
            probe("reinclude_miss", patterns=["//...", "-//personal/...", "//personal:excluded"],
                  expected=["//:a", "//:b", "//personal:excluded"], total=4)
            probe("precise_miss", mode="precise", expected=["//:a", "//:b"])
            assert count("build") == 2, "precise mode did not build the changed dependency"
            run("refresh_baseline", [bazel, "--nosystem_rc", "--nohome_rc", "test", "//..."])
            probe("warm_hits_after_change")
            before = count("test")
            result = run("empty_scope", [aspect, "cache", "diff", "--bazel-startup-flag=--nosystem_rc", "--bazel-startup-flag=--nohome_rc", "--exec=exit 99", "--", "//...", "-//..."])
            assert not result.stdout and count("test") == before
            pattern_file = workspace / "empty-patterns"
            pattern_file.write_text("//...\n-//...\n")
            result = run("empty_pattern_file", [aspect, "cache", "diff", "--bazel-startup-flag=--nosystem_rc", "--bazel-startup-flag=--nohome_rc", f"--target_pattern_file={pattern_file}", "--exec=exit 99"])
            assert not result.stdout and count("test") == before
            assert f"--target_pattern_file={pattern_file}" in result.stderr
            with urllib.request.urlopen(f"http://127.0.0.1:{http_port}/status", timeout=5) as response:
                status = json.load(response)
            print(json.dumps({"timings": timings, "total_seconds": round(time.monotonic() - started, 3),
                              "cache_bytes": status["CurrSize"], "cache_files": status["NumFiles"]}, indent=2), flush=True)
        finally:
            subprocess.run([bazel, "--nosystem_rc", "--nohome_rc", "shutdown"], cwd=workspace, env=env, capture_output=True, timeout=30)


if __name__ == "__main__":
    main()
