## Aspect JavaScript and TypeScript recipe

Use Silo's proven starting trio: `aspect_rules_js` `3.0.1`, `aspect_rules_ts` `3.8.3`, and `rules_nodejs` `6.7.3`. Select the Node and pnpm versions from the repository's existing `package.json`, `.nvmrc`, package-manager configuration, and CI; Silo currently uses Node `22.13.0` and pnpm `9.7.0`. These are a compatibility starting point, not permission to replace a repository's declared runtime or package manager.

Before the first command, check for Bazel or Bazelisk and for the repository's declared package manager (`pnpm`, `yarn`, or `npm`) on `PATH`. The package manager is needed to create or refresh its lockfile; after the Bazel dependency setup works, prefer the Bazel-managed pnpm target for pnpm lockfile maintenance. Check `buildifier` only before formatting BUILD or MODULE files. Do not download, install, or search machine-specific paths for any of these tools. If one is absent, explain which command needs it and ask the user to install the declared version or provide it on `PATH`.

For a pnpm workspace, start `MODULE.bazel` from this shape, adapting paths and versions only after inspecting the repository:

```starlark
bazel_dep(name = "aspect_rules_js", version = "3.0.1")
bazel_dep(name = "aspect_rules_ts", version = "3.8.3")
bazel_dep(name = "rules_nodejs", version = "6.7.3")

node = use_extension("@rules_nodejs//nodejs:extensions.bzl", "node")
node.toolchain(node_version = "<declared-node-version>")
use_repo(node, "nodejs_toolchains")

pnpm = use_extension("@aspect_rules_js//npm:extensions.bzl", "pnpm")
pnpm.pnpm(name = "pnpm", pnpm_version = "<declared-pnpm-version>")
use_repo(pnpm, "pnpm")

npm = use_extension("@aspect_rules_js//npm:extensions.bzl", "npm")
npm.npm_translate_lock(
    name = "npm",
    pnpm_lock = "//:pnpm-lock.yaml",
)
use_repo(npm, "npm")

rules_ts = use_extension("@aspect_rules_ts//ts:extensions.bzl", "ext")
rules_ts.deps(ts_version_from = "//:package.json")
use_repo(rules_ts, "npm_typescript")
```

At the workspace root and every workspace package that needs dependencies, call `npm_link_all_packages(name = "node_modules")`. A first-party workspace package consumed by another workspace must also expose `npm_package(name = "pkg", package = "<package-name>", ...)`; do not assume `package.json` creates that target. Use `js_library`, `js_binary`, and `js_test` for JavaScript, and `ts_project` plus the selected runner or bundler integration for TypeScript. Keep one BUILD file per workspace or source-package directory and use local, non-recursive source lists as the durable layout.

For example, a pnpm workspace package might expose its library and npm package this way; a consuming workspace package then uses the generated local `node_modules` target:

```starlark
# packages/widget/BUILD.bazel
load("@aspect_rules_js//js:defs.bzl", "js_library")
load("@aspect_rules_js//npm:defs.bzl", "npm_package")
load("@npm//:defs.bzl", "npm_link_all_packages")

npm_link_all_packages(name = "node_modules")

js_library(
    name = "widget",
    srcs = ["index.js"],
    deps = [":node_modules/lodash"],
)

npm_package(
    name = "pkg",
    package = "@example/widget",
    srcs = [":widget", "package.json"],
)

# packages/consumer/BUILD.bazel
load("@aspect_rules_js//js:defs.bzl", "js_library")
load("@npm//:defs.bzl", "npm_link_all_packages")

npm_link_all_packages(name = "node_modules")

js_library(
    name = "consumer",
    srcs = ["index.js"],
    deps = [":node_modules/@example/widget"],
)
```

`aspect_rules_js` models pnpm's dependency layout. Do not silently convert a Yarn or npm repository into a pnpm developer workflow. First preserve its declared package-manager workflow and lockfile authority; only introduce a separate Bazel pnpm lock after demonstrating that its resolution, workspace links, patches, overrides, peer dependencies, and lifecycle policy remain equivalent. Treat Yarn `patch:` entries, package-manager plugins, and install scripts as compatibility decisions to report, not syntax to translate optimistically. `npm_translate_lock` must have explicit lifecycle-hook policy; review every package whose install script would run and allow only the necessary scripts. Commit all lock and generated repository-action-cache state required by the selected ruleset.
