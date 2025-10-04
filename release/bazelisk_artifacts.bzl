"""Implementation for `bazelisk_artifacts` rule.

Generates a directory artifact with Aspect CLI Pro binaries named like Bazel
release for Bazelisk downloads.
"""

_ATTRS = {
    "darwin_arm64": attr.label(
        doc = "The artifact for the darwin-arm64 platform.",
        allow_single_file = True,
    ),
    "darwin_x86_64": attr.label(
        doc = "The artifact for the darwin-x86_64 platform.",
        allow_single_file = True,
    ),
    "linux_arm64": attr.label(
        doc = "The artifact for the linux-arm64 platform.",
        allow_single_file = True,
    ),
    "linux_x86_64": attr.label(
        doc = "The artifact for the linux-arm64 platform.",
        allow_single_file = True,
    ),
    "version_file": attr.label(
        doc = "The file that contains the semver of the artifacts.",
        mandatory = True,
        allow_single_file = True,
    ),
    "windows_arm64": attr.label(
        doc = "The artifact for the windows-arm64 platform.",
        allow_single_file = True,
    ),
    "windows_x86_64": attr.label(
        doc = "The artifact for the windows-x86_64 platform.",
        allow_single_file = True,
    ),
    "_sha256sum": attr.label(
        executable = True,
        cfg = "exec",
        default = "//release/sha256sum",
    ),
}

def _impl(ctx):
    outdir = ctx.actions.declare_directory(ctx.label.name)
    inputs = [ctx.file.version_file]
    args = ctx.actions.args()
    args.add(ctx.executable._sha256sum.path)
    args.add(outdir.path)
    args.add(ctx.file.version_file)

    if ctx.attr.windows_x86_64:
        inputs.append(ctx.file.windows_x86_64)
        args.add(ctx.file.windows_x86_64)
        args.add("windows-x86_64.exe")
    elif ctx.attr.windows_arm64:
        inputs.append(ctx.file.windows_arm64)
        args.add(ctx.file.windows_arm64)
        args.add("windows-arm64.exe")
    else:
        inputs.extend([
            ctx.file.darwin_arm64,
            ctx.file.darwin_x86_64,
            ctx.file.linux_arm64,
            ctx.file.linux_x86_64,
        ])
        args.add(ctx.file.darwin_arm64)
        args.add("darwin-arm64")
        args.add(ctx.file.darwin_x86_64)
        args.add("darwin-x86_64")
        args.add(ctx.file.linux_arm64)
        args.add("linux-arm64")
        args.add(ctx.file.linux_x86_64)
        args.add("linux-x86_64")

    ctx.actions.run_shell(
        outputs = [outdir],
        inputs = inputs,
        arguments = [args],
        command = """\
sha256sum="${PWD}/$1"
output_dir="$2"
version_file="$3"
shift 3

version="$(< "${version_file}")"

# Create the output directory
mkdir -p "${output_dir}"

while (("$#")); do
    # Read args in pairs
    artifact_path="$1"
    platform_suffix="$2"
    shift 2

    # Copy the artifact to the output directory
    cp "${artifact_path}" "${output_dir}/bazel-${version}-${platform_suffix}"
    (
        cd "${output_dir}"
        "${sha256sum}" "bazel-${version}-${platform_suffix}" > "bazel-${version}-${platform_suffix}.sha256"
    )
done
""",
        tools = [ctx.executable._sha256sum],
    )

    return [
        DefaultInfo(
            files = depset([outdir]),
            runfiles = ctx.runfiles(files = [outdir]),
        ),
    ]

bazelisk_artifacts = rule(
    implementation = _impl,
    attrs = _ATTRS,
)
