#!/usr/bin/env node
// This script is called by Bazel when it needs info about the git state.
// The --workspace_status_command flag tells Bazel the location of this script.
// This is configured in `/.bazelrc`.
import { execSync } from 'node:child_process';

// "stable" keys, should remain constant over rebuilds, therefore changed values will cause a
// rebuild of any stamped action that uses ctx.info_file or genrule with stamp = True
// Note, BUILD_USER is automatically available in the stable-status.txt, it matches $USER
const has_local_changes = execSync('git status --porcelain').length > 0;
console.log(`STABLE_BUILD_SCM_SHA ${execSync('git rev-parse HEAD')}`);
console.log(
    `STABLE_BUILD_SCM_LOCAL_CHANGES ${has_local_changes ? 'dirty' : 'clean'}`
);

const gitTags = execSync('git tag');
if (gitTags.length > 0) {
    // Follows https://blog.aspect.build/versioning-releases-from-a-monorepo
    const monorepo_version = execSync(
        `git describe --tags --long --match="[0-9][0-9][0-9][0-9].[0-9][0-9]"`,
        { encoding: 'utf-8' }
    )
        .replace('-', '.')
        .replace('-g', '-');

    // Variant of monorepo_version that conforms with the version scheme Bazelisk supports.
    // It assumes the upstream `bazel` binary releases are the only ones referenced,
    // so we are forced to adopt a matching scheme.
    // https://github.com/bazelbuild/bazelisk/blob/47f60477721681a117cbf905784ee5220100e68b/versions/versions.go#L20-L25
    const bazelisk_compat_version = monorepo_version.split('-')[0];
    console.log(
        `STABLE_ASPECT_CLI_BAZELISK_COMPAT_VERSION ${bazelisk_compat_version}`
    );
    console.log(`STABLE_ASPECT_CLI_VERSION ${monorepo_version}`);
}
