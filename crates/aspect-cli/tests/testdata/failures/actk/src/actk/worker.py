#!/usr/bin/env python3
import json, sys
for line in sys.stdin:
    req = json.loads(line)
    args = req.get("arguments", [])
    if args and args[0].startswith("@"):
        args = open(args[0][1:]).read().split()
    out = args[0] if args else ""
    if "fail" in args:
        resp = {"exitCode": 3, "output": "worker says: wk broke on " + out + "\n", "requestId": req.get("requestId", 0)}
    else:
        open(out, "w").write("ok")
        resp = {"exitCode": 0, "output": "", "requestId": req.get("requestId", 0)}
    sys.stdout.write(json.dumps(resp) + "\n"); sys.stdout.flush()
