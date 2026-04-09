#!/usr/bin/env python3
import json
import os
import socket
import sys
import urllib.request

SOCK = "/tmp/circleci-ts.sock"


def get_token():
    s = socket.socket(socket.AF_UNIX, socket.SOCK_STREAM)
    s.connect(SOCK)
    data = b""
    while True:
        chunk = s.recv(4096)
        if not chunk:
            break
        data += chunk
        try:
            return json.loads(data)
        except json.JSONDecodeError:
            continue


def api(token, runner_host, method, path, body=None):
    url = f"{runner_host}{path}"
    data = json.dumps(body).encode() if body else None
    headers = {"Authorization": f"Bearer {token}", "Content-Type": "application/json"}
    req = urllib.request.Request(url, data=data, method=method, headers=headers)
    try:
        with urllib.request.urlopen(req) as resp:
            result = json.loads(resp.read())
            print(f"{method} {path} -> {resp.status}")
            print(json.dumps(result, indent=2))
            return result
    except urllib.error.HTTPError as e:
        print(f"{method} {path} -> {e.code}: {e.read().decode()[:300]}")
        return None


def main():
    creds = get_token()
    token = creds["token"]
    runner_host = creds.get("runner_host", "https://runner.circleci.com")

    # Probe all known /api/v2/output/* endpoints
    api(token, runner_host, "GET",  "/api/v2/output/config")
    print()
    api(token, runner_host, "GET",  "/api/v2/output/credentials")
    print()

    # Post artifact and inspect full response
    result = api(token, runner_host, "POST", "/api/v2/output/artifact", {
        "path": "/tmp/test.txt",
        "destination": "test/hello.txt",
        "artifactType": "text/plain",
    })
    print()
    if result:
        print(f"prefix: {result.get('prefix')}")
        print(f"key.location: {result.get('key', {}).get('location')}")
        print(f"key.tags: {result.get('key', {}).get('tags')}")


if __name__ == "__main__":
    main()
