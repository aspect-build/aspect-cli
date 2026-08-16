# Bazel-ify these container images

Inspect each Dockerfile, compose/deployment configuration, image consumers, base images, system packages, entrypoints, environment, ports, volumes, users, architecture requirements, registry, and CI publishing contract. Treat the Dockerfile as the current behaviour to reproduce, not as input for a blind syntax translation. Preserve it until one representative image has equivalent build, load, and runtime behaviour.

For a new Bazel-owned image, use `rules_img` `0.3.19`, after verifying compatibility with the repository's pinned Bazel version. Prefer `image_from_binary` when the application is already a Bazel `*_binary`; it carries the binary, runfiles, entrypoint, arguments, and run environment into the image. For explicit layout, use `image_layer` and `image_manifest`. Use Bazel target platforms and `image_index` for multi-platform images; do not set an image architecture that disagrees with the binary's build platform.

```starlark
bazel_dep(name = "rules_img", version = "0.3.19")

load("@rules_img//img:image.bzl", "image_from_binary")

image_from_binary(
    name = "image",
    binary = ":server",
    base = "@base_image",
)
```

Treat the runtime as explicit. `image_from_binary` includes the binary's declared runfiles, which may include its hermetic Python interpreter or Java runtime; it does not use or validate a Python or Java already present in the base image. The compiler toolchain used to build the binary is not a runtime dependency to copy into the image. Do not rely on the base runtime unless the binary is deliberately configured and tested against it. Build and run the image to confirm that the selected runtime and application dependencies are present; use runfiles groups when stable runtime files should be separate layers from changing application files.

When a Dockerfile installs Debian or Ubuntu packages, do not run `apt-get` in a Bazel action and do not copy raw `.deb` files into the image. Use `rules_distroless` `0.8.0` to resolve the package manifest to a committed lockfile, then add each package's extracted `:data` tar as a rules_img layer. `rules_pkg`'s `pkg_deb` is for producing a `.deb` artifact; it is not the package installer for an image.

```starlark
# MODULE.bazel
bazel_dep(name = "rules_distroless", version = "0.8.0")

apt = use_extension("@rules_distroless//apt:extensions.bzl", "apt")
apt.install(
    name = "debian",
    manifest = "//images:packages.yaml",
    lock = "//images:packages.lock.json",
)
use_repo(apt, "debian")

# BUILD.bazel
load("@rules_img//img:image.bzl", "image_manifest")

image_manifest(
    name = "image",
    base = "@base_image",
    layers = [
        "@debian//bash/amd64:data",
        "@debian//coreutils/amd64:data",
    ],
)
```

For an image where package-manager metadata matters, generate `/var/lib/dpkg/status` from each installed package's `:control` target with `@rules_distroless//apt:defs.bzl` `dpkg_status`, then include that output as another layer. Select package targets per architecture; do not put an amd64 Debian payload in an arm64 image. Validate package files and metadata with an image structure test, then run a native container test only where a daemon is available.

If parity with a complex Dockerfile is required before its provisioning can be modelled declaratively, retain the Dockerfile temporarily and use a Buildx-based Bazel wrapper as an explicit transitional path. Do not claim it is hermetic or remote-execution-ready. Migrate one concern at a time—base image, application binary, package layers, configuration, and entrypoint—then delete the Dockerfile only after the Bazel image has passed the equivalent load and runtime checks.
