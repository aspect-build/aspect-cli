#!/usr/bin/env python3
import json
import os
import socket
import sys
import tempfile
import urllib.request

SOCK = "/tmp/circleci-ts.sock"


def get_token():
    """Connect to the task socket — it immediately sends token+host as JSON."""
    s = socket.socket(socket.AF_UNIX, socket.SOCK_STREAM)
    s.connect(SOCK)
    data = b""
    while True:
        chunk = s.recv(4096)
        if not chunk:
            break
        data += chunk
        try:
            return json.loads(data)  # stop once we have valid JSON
        except json.JSONDecodeError:
            continue


def main():
    if not os.path.exists(SOCK):
        print(f"Socket not found: {SOCK}")
        sys.exit(1)

    creds = get_token()
    token = creds["token"]
    runner_host = creds.get("runner_host", "https://runner.circleci.com")
    print(f"token: {token[:8]}...{token[-4:]}")
    print(f"runner_host: {runner_host}")

    test_content = b"hello from socket test\n"
    with tempfile.NamedTemporaryFile(delete=False, suffix=".txt") as f:
        f.write(test_content)
        tmp_path = f.name

    try:
        body = json.dumps({
            "path": tmp_path,
            "destination": "test/hello.txt",
            "artifactType": "text/plain",
        }).encode()

        req = urllib.request.Request(
            f"{runner_host}/api/v2/output/artifact",
            data=body,
            method="POST",
            headers={
                "Authorization": f"Bearer {token}",
                "Content-Type": "application/json",
            },
        )
        with urllib.request.urlopen(req) as resp:
            result = json.loads(resp.read())
            print(f"\nArtifact location response:\n{json.dumps(result, indent=2)}")

        upload_url = result.get("url") or result.get("location") or result.get("upload_url")
        if upload_url:
            req2 = urllib.request.Request(upload_url, data=test_content, method="PUT",
                headers={"Content-Type": "text/plain"})
            with urllib.request.urlopen(req2) as resp:
                print(f"\nUpload status: {resp.status} — Success!")
        else:
            print(f"Unknown response shape, fields: {list(result.keys())}")
    except urllib.error.HTTPError as e:
        print(f"HTTP Error {e.code}: {e.read().decode()}")
    finally:
        os.unlink(tmp_path)


if __name__ == "__main__":
    main()
