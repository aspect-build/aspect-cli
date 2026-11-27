"""Scrub one recorded run and copy the files its BEP refers to.

usage: fxpost.py <fixture_dir> <prefix> <raw_dir> <output_base> <install_base> <workspace> <fake_home>

Reads <raw_dir>/{bep.json,console,stdout,exit}; writes <fixture_dir>/<prefix>.{bep.json,console,
stdout,exit} and copies every action stderr/stdout and test.log/test.xml the BEP names into
<fixture_dir>/<prefix>.files/<path relative to the output base>. The BEP is trimmed of the
events the failure summary never reads (`DROPPED`): progress chunks, the command lines, the
build tool logs; an Aborted one of those stays.
"""
import json, os, re, shutil, socket, sys, getpass, urllib.parse
fx, prefix, raw, ob, ib, ws, fakehome = sys.argv[1:8]
home = os.path.expanduser("~")
def spellings(p):  # macOS reports /private/var and /private/tmp paths both ways
    return [p, p[len("/private"):]] if p.startswith("/private/") else [p]
subs = []
for p, name in ((ib, "${INSTALL_BASE}"), (ob, "${OUTPUT_BASE}"), (os.path.dirname(ob), "${OUTPUT_USER_ROOT}")):
    subs += [(x, name) for x in spellings(p)]
for w in sorted({ws, os.path.realpath(ws), ws.replace("/private/tmp/", "/tmp/", 1)}, key=len, reverse=True):
    subs.append((w, "${WORKSPACE}"))
subs.append((raw, "${RAW}"))
subs.append((fakehome, "${HOME}"))
subs.append((home, "${HOME}"))
user = getpass.getuser()
host = socket.gethostname()
def scrub(s):
    for a, b in subs:
        if a: s = s.replace(a, b)
    s = re.sub(r"/var/folders/[^/]+/[^/]+/", "${DARWIN_USER_DIR}/", s)
    for k in ("USER", "LOGNAME"):
        s = s.replace(k + "=" + user, k + "=${USER}")
    return s
def scrub_status(e):
    # The fields that carry the host and user as values, by name: the host
    # is scrubbed there alone, never in text or a path, where a short name
    # like `build` is an ordinary word (a path under the host's name is
    # under one the path scrubs above already cover). `guard` checks that
    # none is left.
    started = e.get("started", {})
    if "user" in started: started["user"] = "${USER}"
    if "host" in started: started["host"] = "${HOST}"
    info = e.get("testResult", {}).get("executionInfo", {})
    if "hostname" in info: info["hostname"] = "${HOST}"
    for it in e.get("workspaceStatus", {}).get("item", []):
        if it.get("key") == "BUILD_USER": it["value"] = "${USER}"
        if it.get("key") == "BUILD_HOST": it["value"] = "${HOST}"
    return e
def guard(paths):
    """Where this machine's host (short and full), user or home is left in
    `paths`: as a whole word or a path component, never inside another
    word. A name in FIXTURE_GUARD_ALLOW (comma-separated) may stand as a
    whole word of text (a host or user named `build`), never in a path."""
    allow = [a for a in os.environ.get("FIXTURE_GUARD_ALLOW", "").split(",") if a]
    names = sorted({n for n in (host, host.split(".")[0], user) if n}, key=len, reverse=True)
    found = []
    for p in paths:
        text = open(p, "rb").read().decode("utf-8", "surrogateescape")
        for h in spellings(home):
            if re.search(re.escape(h) + r"(?![\w.-])", text) or (h + "/") in text:
                found.append((p, h))
        for n in names:
            word = r"(?<![\w.-])" + re.escape(n) + r"(?![\w-])"
            for m in re.finditer(word, text):
                in_path = (m.start() > 0 and text[m.start() - 1] == "/") or text[m.end():m.end() + 1] == "/"
                if in_path or n not in allow:
                    found.append((p, n))
                    break
    return found
def copy(uri):
    if not uri.startswith("file://" + ob + "/"): return
    # A URI's path is percent-encoded, and Bazel spells a non-ASCII name's UTF-8 bytes as
    # Latin-1 characters in it (`Ñ%C2%82` for `т`); the file on disk has one spelling or
    # the other (Bazel 9 names it as the URI does, Bazel 8 as meant). Kept as found.
    rel = urllib.parse.unquote(uri[len("file://" + ob + "/"):])
    if not os.path.exists(os.path.join(ob, rel)):
        try:
            rel = rel.encode("latin-1").decode("utf-8")
        except (UnicodeEncodeError, UnicodeDecodeError):
            pass
    dst = os.path.join(fx, prefix + ".files", rel)
    try:
        os.makedirs(os.path.dirname(dst), exist_ok=True); shutil.copyfile(os.path.join(ob, rel), dst)
    except OSError as x:
        print("fxpost: cannot copy", rel, x, file=sys.stderr)
# Event kinds the failure summary never reads; they are most of a fixture's bytes.
DROPPED = ("progress", "structuredCommandLine", "unstructuredCommandLine", "buildToolLogs")
def kept(e):
    return "aborted" in e or list(e["id"])[0] not in DROPPED
out = []
bep = os.path.join(raw, "bep.json")
if os.path.exists(bep):
    for line in open(bep):
        e = json.loads(line)
        a = e.get("action", {})
        for k in ("stderr", "stdout"):
            if k in a: copy(a[k].get("uri", ""))
        for f in e.get("testResult", {}).get("testActionOutput", []):
            if f.get("name") in ("test.log", "test.xml"): copy(f.get("uri", ""))
        if not kept(e): continue
        out.append(scrub(json.dumps(scrub_status(e), ensure_ascii=False)))
    open(os.path.join(fx, prefix + ".bep.json"), "w").write("\n".join(out) + "\n")
for k in ("console", "stdout", "exit"):
    p = os.path.join(raw, k)
    if not os.path.exists(p): continue
    data = open(p, "rb").read().decode("utf-8", "surrogateescape")
    if k == "stdout" and not data: continue
    if k == "console" and data.startswith("^D"):
        data = data[2:].lstrip("\b")  # BSD script(1) echoes EOF of its /dev/null stdin
    open(os.path.join(fx, prefix + "." + k), "w", encoding="utf-8", errors="surrogateescape").write(scrub(data))
filesdir = os.path.join(fx, prefix + ".files")
for root, _, names in os.walk(filesdir):
    for n in names:
        p = os.path.join(root, n)
        b = open(p, "rb").read().decode("utf-8", "surrogateescape")
        open(p, "w", encoding="utf-8", errors="surrogateescape").write(scrub(b))
written = [os.path.join(fx, prefix + "." + k) for k in ("bep.json", "console", "stdout", "exit")]
written = [p for p in written if os.path.exists(p)]
for root, _, names in os.walk(filesdir):
    written += [os.path.join(root, n) for n in names]
left = guard(written)
for p, name in left:
    print("fxpost: %s still names this machine (%r)" % (p, name), file=sys.stderr)
if left:
    sys.exit(3)
