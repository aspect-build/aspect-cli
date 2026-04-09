#!/usr/bin/env python3
"""
Test script for CircleCI's /api/v2/output/artifact endpoint.
Must be run inside a CircleCI job where CCI_RUNNER_API_TASK_TOKEN is set.
"""

import json
import os
import sys
import tempfile
import urllib.request

TOKEN = os.environ.get("CCI_RUNNER_API_TASK_TOKEN")
BASE_URL = os.environ.get("CCI_RUNNER_API_BASE_URL", "https://runner.circleci.com")
ARTIFACT_ENDPOINT = f"{BASE_URL}/api/v2/output/artifact"


def request_artifact_location(path: str, destination: str, content_type: str = "text/plain") -> dict:
    """POST to /api/v2/output/artifact to get a presigned upload URL."""
    body = json.dumps({
        "path": path,
        "destination": destination,
        "artifactType": content_type,
    }).encode()

    req = urllib.request.Request(
        ARTIFACT_ENDPOINT,
        data=body,
        method="POST",
        headers={
            "Authorization": f"Bearer {TOKEN}",
            "Content-Type": "application/json",
        },
    )

    print(f"POST {ARTIFACT_ENDPOINT}")
    print(f"  Body: {body.decode()}")

    try:
        with urllib.request.urlopen(req) as resp:
            status = resp.status
            raw = resp.read()
            print(f"  Status: {status}")
            data = json.loads(raw)
            print(f"  Response: {json.dumps(data, indent=2)}")
            return data
    except urllib.error.HTTPError as e:
        print(f"  HTTP Error {e.code}: {e.reason}")
        print(f"  Body: {e.read().decode()}")
        sys.exit(1)


def upload_to_presigned_url(url: str, content: bytes, content_type: str = "text/plain"):
    """PUT file bytes directly to the presigned S3 URL."""
    req = urllib.request.Request(
        url,
        data=content,
        method="PUT",
        headers={"Content-Type": content_type},
    )

    print(f"\nPUT {url[:80]}...")
    try:
        with urllib.request.urlopen(req) as resp:
            print(f"  Status: {resp.status}")
    except urllib.error.HTTPError as e:
        print(f"  HTTP Error {e.code}: {e.reason}")
        print(f"  Body: {e.read().decode()}")
        sys.exit(1)


def main():
    if not TOKEN:
        print("ERROR: CCI_RUNNER_API_TASK_TOKEN is not set.")
        print("This script must be run inside a CircleCI job.")
        sys.exit(1)

    print(f"Token: {TOKEN[:8]}...{TOKEN[-4:]}")
    print(f"Base URL: {BASE_URL}\n")

    # Create a small test file
    test_content = b"Hello from store_artifacts test!\n"
    with tempfile.NamedTemporaryFile(delete=False, suffix=".txt") as f:
        f.write(test_content)
        tmp_path = f.name

    try:
        # Step 1: Get presigned upload location
        location = request_artifact_location(
            path=tmp_path,
            destination="test/hello.txt",
        )

        # Step 2: Upload to presigned URL (look for url/location/key fields)
        upload_url = location.get("url") or location.get("location") or location.get("upload_url")
        if not upload_url:
            print(f"\nUnexpected response shape — full response above. Fields: {list(location.keys())}")
            sys.exit(0)

        upload_to_presigned_url(upload_url, test_content)
        print("\nSuccess!")
    finally:
        os.unlink(tmp_path)


if __name__ == "__main__":
    main()
