"""
A library of aspects used by run--watch.

Keep MINIMAL and AVOID UNNECESSARY CHANGES as this is written to disk and maybe never be cleaned.
"""

def _watch_manifest_impl(target, ctx):
    default = target[DefaultInfo]
    target_path = default.files_to_run.executable.short_path.removeprefix(target.label.package + "/")

    watch_manifest = ctx.actions.declare_file("{}.watch_manifest".format(target_path))

    # This target is allowed to be non-hermetic as its only generated in conjuction with --watch
    # and expected to contain absolute paths.
    ctx.actions.run_shell(
        command = """#!/usr/bin/env bash
# Write execroot to the info file
pwd > {info_file};

# Write the data passed as args to the info file
for info in "$@"; do
    echo "$info" >> {info_file};
done

# Copy the manifest file to the global temp location
cp {info_file} {aspect_watch_watch_manifest}
""".format(
            info_file = watch_manifest.path,
            aspect_watch_watch_manifest = ctx.attr.aspect_watch_watch_manifest,
        ),
        arguments = [
            default.files_to_run.executable.path,
            str(ctx.label),
            " ".join(ctx.rule.attr.tags),
        ],
        outputs = [watch_manifest],
        mnemonic = "AspectWatchTargetInfo",
        execution_requirements = {
            # prevents the action or test from being executed remotely or cached remotely
            "no-remote": "1",
            # keyword results in the action or test never being cached (locally or remotely)
            "no-cache": "1",
            # precludes the action from being remotely cached, remotely executed, or run inside the sandbox
            "local": "1",
        },
    )

    return [OutputGroupInfo(
        __aspect_watch_watch_manifest = depset([watch_manifest]),
    )]

watch_manifest = aspect(
    implementation = _watch_manifest_impl,
    attr_aspects = [],
    required_providers = [DefaultInfo],
    attrs = {
        "aspect_watch_watch_manifest": attr.string(),
    },
)
