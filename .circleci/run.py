#!/usr/bin/env python3
import http.client
import json
import os
import socket
import sys
import tempfile


class UnixSocketHTTPConnection(http.client.HTTPConnection):
    def __init__(self, socket_path):
        super().__init__("localhost")
        self.socket_path = socket_path

    def connect(self):
        self.sock = socket.socket(socket.AF_UNIX, socket.SOCK_STREAM)
        self.sock.connect(self.socket_path)


SOCK = "/tmp/circleci-ts.sock"


def call(method, path, body=None):
    conn = UnixSocketHTTPConnection(SOCK)
    headers = {"Content-Type": "application/json"} if body else {}
    data = json.dumps(body).encode() if body else None
    conn.request(method, path, body=data, headers=headers)
    resp = conn.getresponse()
    raw = resp.read()
    print(f"{method} {path} -> {resp.status}")
    try:
        parsed = json.loads(raw)
        print(json.dumps(parsed, indent=2))
        return parsed
    except Exception:
        print(raw.decode()[:500])
        return None


def main():
    if not os.path.exists(SOCK):
        print(f"Socket not found: {SOCK}")
        sys.exit(1)

    # Probe what's available
    for path in ["/", "/api/v2/output/artifact", "/v2/task/config",
                 "/task-agent-subcommands", "/api"]:
        call("GET", path)
        print()

    # Try posting an artifact
    test_content = b"hello from socket test\n"
    with tempfile.NamedTemporaryFile(delete=False, suffix=".txt") as f:
        f.write(test_content)
        tmp_path = f.name

    print("--- POST /api/v2/output/artifact ---")
    result = call("POST", "/api/v2/output/artifact", {
        "path": tmp_path,
        "destination": "test/hello.txt",
        "artifactType": "text/plain",
    })
    os.unlink(tmp_path)

    if result:
        upload_url = result.get("url") or result.get("location") or result.get("upload_url")
        print(f"\nUpload URL: {upload_url}")


if __name__ == "__main__":
    main()
