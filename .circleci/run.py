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
    with urllib.request.urlopen(req) as resp:
        return json.loads(resp.read())


def upload_artifact(file_path, destination, file_content=None):
    import boto3

    creds = get_token()
    token = creds["token"]
    runner_host = creds.get("runner_host", "https://runner.circleci.com")

    config = api(token, runner_host, "GET", "/api/v2/output/config")
    bucket = config["bucket"]
    region = config["region"]

    aws_creds = api(token, runner_host, "GET", "/api/v2/output/credentials")["s3"]

    artifact_resp = api(token, runner_host, "POST", "/api/v2/output/artifact", {
        "path": file_path,
        "destination": destination,
        "artifactType": "text/plain",
    })
    prefix = artifact_resp["prefix"]
    tags = artifact_resp["key"].get("tags", {})

    s3_key = f"{prefix}/{destination}"
    tag_str = "&".join(f"{k}={v}" for k, v in tags.items())

    print(f"Uploading to s3://{bucket}/{s3_key}")

    s3 = boto3.client(
        "s3",
        region_name=region,
        aws_access_key_id=aws_creds["AccessKeyID"],
        aws_secret_access_key=aws_creds["SecretAccessKey"],
        aws_session_token=aws_creds["SessionToken"],
    )

    content = file_content or open(file_path, "rb").read()
    s3.put_object(
        Bucket=bucket,
        Key=s3_key,
        Body=content,
        ContentType="text/plain",
        Tagging=tag_str,
    )
    print(f"Done. Artifact at: s3://{bucket}/{s3_key}")


if __name__ == "__main__":
    upload_artifact("/tmp/test.txt", "test/hello.txt", b"hello from direct S3 upload!\n")
