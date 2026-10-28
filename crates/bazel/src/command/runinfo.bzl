_BS = "\\"

def _tokenize(s):
    tokens = []
    token = ""
    active = False
    i = 0
    n = len(s)
    for _ in range(n + 1):
        if i >= n:
            break
        c = s[i]
        i += 1
        if c == "'":
            active = True
            closed = False
            for _ in range(n + 1):
                if i >= n:
                    break
                d = s[i]
                i += 1
                if d == "'":
                    closed = True
                    break
                token += d
            if not closed:
                fail("tokenize: unterminated single quote in: " + s)
        elif c == '"':
            active = True
            closed = False
            for _ in range(n + 1):
                if i >= n:
                    break
                d = s[i]
                i += 1
                if d == '"':
                    closed = True
                    break
                if d == _BS:
                    if i >= n:
                        fail("tokenize: trailing backslash in: " + s)
                    e = s[i]
                    i += 1
                    if e == '"' or e == _BS:
                        token += e
                    else:
                        token += _BS + e
                else:
                    token += d
            if not closed:
                fail("tokenize: unterminated double quote in: " + s)
        elif c == " " or c == "\t":
            if active:
                tokens.append(token)
                token = ""
                active = False
        elif c == _BS:
            if i >= n:
                fail("tokenize: trailing backslash in: " + s)
            token += s[i]
            i += 1
            active = True
        else:
            token += c
            active = True
    if active:
        tokens.append(token)
    return tokens

def _expand_and_tokenize_args(ctx, target):
    raw = getattr(ctx.rule.attr, "args", None)
    if not raw:
        return []
    lookup = getattr(ctx.rule.attr, "data", []) + [target]
    out = []
    for a in raw:
        # `bazel run` expands `args` with LocationExpander.withRunfilesPaths, so
        # `$(location X)` there is X's runfiles-relative path, i.e. `$(rootpath X)`.
        # From user code ctx.expand_location only offers the execpath flavour
        # (`short_paths = True` is private-API gated), so rewrite the template
        # names first. Bazel recognises a template by the literal "$(<name> ",
        # so this substitution covers exactly the templates it would expand.
        a = a.replace("$(locations ", "$(rootpaths ").replace("$(location ", "$(rootpath ")
        a = ctx.expand_location(a, lookup)
        a = ctx.expand_make_variables("args", a, {})
        out.extend(_tokenize(a))
    return out

def _runinfo_aspect_impl(target, ctx):
    files_to_run = target[DefaultInfo].files_to_run
    if files_to_run == None or files_to_run.executable == None:
        return []

    env = {}
    inherited = []
    if RunEnvironmentInfo in target:
        rei = target[RunEnvironmentInfo]
        env = rei.environment
        inherited = rei.inherited_environment

    # Whether the target has runfiles support at all, and whether Bazel wrote
    # the input manifest for it. A target with runfiles but no manifest means
    # --nobuild_runfile_manifests is in effect, which nothing can run.
    runfiles = target[DefaultInfo].default_runfiles
    info = struct(
        executable = files_to_run.executable.short_path,
        args = _expand_and_tokenize_args(ctx, target),
        environment = env,
        inherited_environment = inherited,
        tags = getattr(ctx.rule.attr, "tags", []),
        runfiles = runfiles != None and len(runfiles.files.to_list()) > 0,
        runfiles_manifest = files_to_run.runfiles_manifest != None,
    )

    out = ctx.actions.declare_file(target.label.name + ".aspect_runinfo.json")
    ctx.actions.write(out, json.encode(info))
    return [OutputGroupInfo(aspect_runinfo = depset([out]))]

runinfo = aspect(implementation = _runinfo_aspect_impl)
