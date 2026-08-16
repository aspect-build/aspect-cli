# Migrate these Bazel images from rules_oci to rules_img

This is an incremental migration of an existing Bazel container-image build, not a Dockerfile conversion or a Bzlmod migration. Inspect the current module or workspace declarations, image pulls, layers, image and index targets, load and push targets, output-group consumers, structure tests, platform definitions, registry policy, and CI before changing anything. Preserve working image labels and publishing semantics during the first stage.

Add `rules_img` `0.3.19` beside `rules_oci`; do not remove `rules_oci`, `rules_pkg`, or tar-producing targets merely because a replacement is available. Migrate one leaf image and every consumer of its outputs, then retain an OCI-layout bridge where an unmigrated target must consume it. Do not combine this work with a Bazel-major or WORKSPACE-to-Bzlmod migration.

Replace the `oci.pull()` module extension with the `rules_img` `pull()` repository rule. Its image address is split into `registry` and `repository`; keep the existing digest and registry-mirror policy. `pull()` creates the repository directly, so it needs neither `use_repo()` nor a manually maintained platform list. Set `layer_handling = "eager"` to match what `oci.pull()` did: the default `"shallow"` records layer descriptors without fetching the blobs, and every consumer that needs real bytes — the `oci_layout` and `oci_tarball` output groups, an OCI-layout bridge, `container_structure_test` — then fails with `Missing layer blobs`. A `pull()` name is a root-module repository name, so while both rulesets coexist it cannot reuse the name an `oci.pull()` already holds; rename the new repository and update its consumers' `base`, rather than renaming the old one:

```starlark
bazel_dep(name = "rules_img", version = "0.3.19")

pull = use_repo_rule("@rules_img//img:pull.bzl", "pull")
pull(
    name = "distroless_cc",
    digest = "sha256:<pinned-digest>",
    layer_handling = "eager",
    registry = "gcr.io",
    repository = "distroless/cc-debian12",
)
```

Migrate image assembly deliberately: `oci_image` becomes `image_manifest`, and `tars` becomes `layers`. Existing `pkg_tar` or tar.bzl targets may stay as layers for the first slice; move to `image_layer` only when its explicit paths and metadata improve the result. Preserve `base`, `entrypoint`, `cmd`, dictionary-valued `env`, labels, annotations, user, and timestamps. Rename `workdir` to `working_dir`; replace an `env` file label with `env_file`; model unsupported `exposed_ports` or `volumes` through `config_fragment`. Do not manually set image `os` or `architecture`: rules_img derives them from the Bazel target platform.

```starlark
load("@rules_img//img:image.bzl", "image_manifest")

image_manifest(
    name = "app_image",
    base = "@distroless_cc",
    layers = [":app_layer"],
    entrypoint = ["/app/bin/server"],
)
```

Migrate `oci_image_index` to `image_index`, renaming its `images` attribute to `manifests`. Prefer one manifest definition plus the index's platform transitions, so application binaries and layers are rebuilt for each declared platform; do not copy one amd64 manifest into an arm64 index. If an unmigrated rules_oci target needs a rules_img image, expose its `oci_layout` output group through a filegroup. In the other direction, use `image_manifest_from_oci_layout` or `image_index_from_oci_layout` from `@rules_img//img:convert.bzl`, with the real OS, architecture, media types, and manifests. Treat either bridge as temporary migration state, not the final image architecture.

Update output consumers with the image target, not by assuming that its default output remains an OCI layout: `image_index` defaults to an index JSON file, not a directory, and its `oci_layout` output group lists each platform's manifest directly where rules_oci nested them under an inner index. Use the `oci_layout` or `oci_tarball` output group where a downstream action needs that format, and re-read any consumer that parses `index.json` itself. Replace `oci_load` with `image_load` and `repo_tags` with `tag`, `tag_list`, or `tag_file`; replace `oci_push` with `image_push`, splitting its repository address into `registry` and `repository`, and map `remote_tags` to `tag` or `tag_list` — or, where it is a label rather than a list, to `tag_file`, so an existing stamped-tag template keeps producing the published tag names. Preserve the existing authentication and publishing policy. Note that a shallow base-image pull moves base-layer fetching from build time to push time, so the pushing environment needs its own access to the upstream registry; keep `eager` unless that access is established.

Push and image rules are frequently wrapped in a repository's own release macro rather than written out per package, in which case one leaf image cannot be migrated without touching every image the macro serves. Parameterise the macro with a flag selecting the rules_img rule, defaulted to the rules_oci path, so unmigrated callers are unaffected and the flag can be deleted with the last one; do not fork the macro, and do not migrate every caller at once to avoid the flag.

Validate each migrated slice by building the image, exercising its existing structure test against the required output format, and loading or pushing only through the repository's approved environment. Confirm image configuration, files, entrypoint, labels, platform manifests, and tag names against the prior image. Where a slice has no structure test, decode both OCI layouts and compare layer digests and file listings directly; keeping existing tar layers unchanged is what makes those digests match, and a mismatch there is the earliest signal that the migration altered content. Expect the image configuration to differ by the legacy Docker fields rules_oci carried and by added base-image annotations, and expect the resulting image digest to change. Report any consumer still relying on a rules_oci layout bridge. Remove rules_oci and tar-layer compatibility only after every image, index, load/push target, test, and CI platform has passed.
