"""Custom executable rules for delivery e2e testing.

Both rules use a non-standard executable file extension (.sh and .bash), which
exercises the naming fix: Bazel places the runfiles tree at <exec>.runfiles
(after the executable file, including extension), not <target_name>.runfiles.
"""

def _custom_deliverable_impl(ctx):
    runner = ctx.actions.declare_file(ctx.label.name + ".sh")
    ctx.actions.write(
        output = runner,
        is_executable = True,
        content = """\
#!/usr/bin/env bash
set -euo pipefail
runfiles_dir="${RUNFILES_DIR:-$0.runfiles}"
manifest_file="${RUNFILES_MANIFEST_FILE:-}"

rlocation() {
  local path="$1"
  if [[ -n "$manifest_file" && -f "$manifest_file" ]]; then
    awk -v p="$path" '$1 == p { print substr($0, length($1)+2); found=1; exit } END { if (!found) exit 1 }' "$manifest_file"
  else
    printf '%s/%s\n' "$runfiles_dir" "$path"
  fi
}

echo "custom_deliverable..."
payload="$(rlocation "_main/examples/deliverable/delivery_payload.txt")"
echo "custom_deliverable ran; payload contents: $(cat "$payload")"
""",
    )
    return [DefaultInfo(
        files = depset([runner]),
        executable = runner,
        runfiles = ctx.runfiles(files = ctx.files.data),
    )]

custom_deliverable = rule(
    implementation = _custom_deliverable_impl,
    executable = True,
    attrs = {"data": attr.label_list(allow_files = True)},
)

def _oci_like_deliverable_impl(ctx):
    """Sim of `rules_oci`'s push rule: an executable plus two sibling
    default outputs (.digest and .layer), where only .digest is exposed
    via runfiles. .layer reaches disk via `--remote_download_outputs=toplevel`
    (materializing the target's default outputs), but is NOT reachable
    from inside `runfiles_dir`. The customer hook in `.aspect/config.axl`
    must locate it via the entry's `default_outputs` list.
    """
    runner = ctx.actions.declare_file(ctx.label.name)
    digest = ctx.actions.declare_file(ctx.label.name + ".digest")
    layer = ctx.actions.declare_file(ctx.label.name + ".layer")

    ctx.actions.write(
        output = runner,
        is_executable = True,
        content = "#!/usr/bin/env bash\necho 'oci_like_deliverable ran'\n",
    )
    ctx.actions.write(
        output = digest,
        content = "sha256:0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef\n",
    )
    ctx.actions.write(
        output = layer,
        content = "layer-blob-placeholder\n",
    )
    return [DefaultInfo(
        files = depset([runner, digest, layer]),
        executable = runner,
        runfiles = ctx.runfiles(files = [digest]),
    )]

oci_like_deliverable = rule(
    implementation = _oci_like_deliverable_impl,
    executable = True,
)

def _bash_deliverable_impl(ctx):
    runner = ctx.actions.declare_file(ctx.label.name + ".bash")
    ctx.actions.write(
        output = runner,
        is_executable = True,
        content = "#!/usr/bin/env bash\necho 'bash_deliverable ran'\n",
    )
    return [DefaultInfo(
        files = depset([runner]),
        executable = runner,
        runfiles = ctx.runfiles(files = ctx.files.data),
    )]

bash_deliverable = rule(
    implementation = _bash_deliverable_impl,
    executable = True,
    attrs = {"data": attr.label_list(allow_files = True)},
)
