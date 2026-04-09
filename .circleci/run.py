#!/usr/bin/env python3
import json
import os
import stat
import sys
import tempfile
import urllib.request

TOKEN = os.environ.get("CCI_RUNNER_API_TASK_TOKEN")
BASE_URL = os.environ.get("CCI_RUNNER_API_BASE_URL", "https://runner.circleci.com")
ARTIFACT_ENDPOINT = f"{BASE_URL}/api/v2/output/artifact"


def explore_path(label, path):
    print(f"\n=== {label} ===")
    if not path:
        print("  Not set")
        return
    print(f"  Path: {path}")
    if not os.path.exists(path):
        print("  Does not exist")
        return
    st = os.stat(path)
    mode = st.st_mode
    kind = ('dir' if stat.S_ISDIR(mode) else
            'file' if stat.S_ISREG(mode) else
            'socket' if stat.S_ISSOCK(mode) else 'other')
    print(f"  Type: {kind}, size: {st.st_size}")
    if stat.S_ISDIR(mode):
        for fname in sorted(os.listdir(path)):
            fpath = os.path.join(path, fname)
            fst = os.stat(fpath)
            fmode = fst.st_mode
            if stat.S_ISREG(fmode):
                try:
                    with open(fpath) as f:
                        print(f"  [{fname}]: {f.read()[:1000]}")
                except Exception as e:
                    print(f"  [{fname}]: (unreadable: {e})")
            else:
                fkind = 'socket' if stat.S_ISSOCK(fmode) else 'dir' if stat.S_ISDIR(fmode) else 'other'
                print(f"  [{fname}]: ({fkind})")
    elif stat.S_ISREG(mode):
        try:
            with open(path) as f:
                print(f"  Contents: {f.read()[:2000]}")
        except Exception as e:
            print(f"  (unreadable: {e})")


def main():
    print("=== Environment ===")
    for k, v in sorted(os.environ.items()):
        if any(k.startswith(p) for p in ("CCI_", "CIRCLE_", "CIRCLECI")):
            display = v if len(v) < 40 else f"{v[:8]}...{v[-4:]}"
            print(f"  {k}={display}")

    explore_path("CIRCLE_INTERNAL_TASK_DATA", os.environ.get("CIRCLE_INTERNAL_TASK_DATA"))
    explore_path("CIRCLE_INTERNAL_SCRATCH", os.environ.get("CIRCLE_INTERNAL_SCRATCH"))

    print("\n=== searching for circleci sockets/pids ===")
    search_paths = [
        "/tmp/circle-agent-runner.pid", "/run/circleci-agent.sock",
        "/tmp/.circleci-agent", "/var/run/circleci-agent",
        "/tmp/circleci-ts.sock", "/run/circleci-ts.sock",
    ]
    scratch = os.environ.get("CIRCLE_INTERNAL_SCRATCH", "")
    if scratch and os.path.isdir(scratch):
        for fname in os.listdir(scratch):
            search_paths.append(os.path.join(scratch, fname))
    for path in search_paths:
        if os.path.exists(path):
            st = os.stat(path)
            kind = ('socket' if stat.S_ISSOCK(st.st_mode) else
                    'file' if stat.S_ISREG(st.st_mode) else 'dir')
            print(f"  FOUND {path} ({kind})")

    print()
    if not TOKEN:
        print("CCI_RUNNER_API_TASK_TOKEN is not set — see diagnostics above.")
        sys.exit(1)

    print(f"Token: {TOKEN[:8]}...{TOKEN[-4:]}")
    print(f"Base URL: {BASE_URL}\n")

    test_content = b"Hello from store_artifacts test!\n"
    with tempfile.NamedTemporaryFile(delete=False, suffix=".txt") as f:
        f.write(test_content)
        tmp_path = f.name

    try:
        body = json.dumps({"path": tmp_path, "destination": "test/hello.txt", "artifactType": "text/plain"}).encode()
        req = urllib.request.Request(ARTIFACT_ENDPOINT, data=body, method="POST",
            headers={"Authorization": f"Bearer {TOKEN}", "Content-Type": "application/json"})
        with urllib.request.urlopen(req) as resp:
            data = json.loads(resp.read())
            print(f"Response: {json.dumps(data, indent=2)}")

        upload_url = data.get("url") or data.get("location") or data.get("upload_url")
        if upload_url:
            req2 = urllib.request.Request(upload_url, data=test_content, method="PUT",
                headers={"Content-Type": "text/plain"})
            with urllib.request.urlopen(req2) as resp:
                print(f"Upload status: {resp.status}")
        else:
            print(f"Unknown response shape, fields: {list(data.keys())}")
    finally:
        os.unlink(tmp_path)


if __name__ == "__main__":
    main()
