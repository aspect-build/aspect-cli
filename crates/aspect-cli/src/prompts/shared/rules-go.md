## rules_go and Gazelle

Use the current `rules_go` and Gazelle baseline only after confirming that it supports the repository's pinned Bazel version. This catalogue pins `rules_go` `0.62.0` and Gazelle `0.52.2`; do not substitute an unverified newer release or alter the Bazel major as part of this work.

Before validation, ensure the available Bazel/Bazelisk resolves the `.bazelversion` value. If only a different plain `bazel` is available, continue inspection and drafting, but stop before validation or label it explicitly non-authoritative; do not claim the pinned baseline passed under another Bazel version.

For a single-module repository, begin `MODULE.bazel` with this baseline. Replace the module name with a stable Bazel module identifier; set the Gazelle prefix below to the `module` path from `go.mod`, not the Bazel module name.

```starlark
module(name = "<bazel-module-name>")

bazel_dep(name = "rules_go", version = "0.62.0")
bazel_dep(name = "gazelle", version = "0.52.2")

go_sdk = use_extension("@rules_go//go:extensions.bzl", "go_sdk")
go_sdk.from_file(go_mod = "//:go.mod")
# Or, where go.mod declares only `go <major>.<minor>`:
# go_sdk.download(version = "<major>.<minor>.<patch>")

go_deps = use_extension("@gazelle//:extensions.bzl", "go_deps")
go_deps.from_file(go_mod = "//:go.mod")
```

`go_sdk.from_file` resolves the `go` or `toolchain` directive verbatim against the published release list, which contains only full patch releases. A `go.mod` declaring a bare `go <major>.<minor>` with no `toolchain` line therefore fails with `did not find version`. Either use `go_sdk.download` with the patch release the repository actually builds with, or add a `toolchain` line to `go.mod` and report it as a source change.

Create the root `BUILD.bazel` before evaluating these extensions. Add the root Gazelle target and prefix directive with the same Go module path. For a `go.work` workspace, first identify each independent Go module. An initial slice may model only the module that owns the selected package; record omitted modules and add a `from_file` declaration for each before claiming workspace completion. Do not point every module at the root by assumption. Run Gazelle, then `bazel mod tidy`, then build: `mod tidy` reads the generated BUILD files to derive `use_repo`, and hand-transcribing those names from the extension's warning output is wasted work. If dependency or release-note access is unavailable, record the selected pair as unverified and stop before declaring it compatible; do not spend unbounded time researching before the local scaffold exists.

Gazelle emits neither `size` nor `timeout` on generated `go_test` targets, so every test lands on the default moderate timeout. Set them where the repository's tests are known to be faster or slower than that.

For the first target, add only the `use_repo(go_deps, ...)` repositories that its generated dependencies reference. Once that target builds, `bazel mod tidy` can derive the complete set. The shared `.bazelrc` baseline is a completion-policy proposal: do not let it delay proving the first target, and add it before CI/remote configuration rather than copying broad policy into an exploratory slice.

```starlark
load("@gazelle//:def.bzl", "gazelle")

# gazelle:prefix <go-module-path-from-go.mod>
gazelle(
    name = "gazelle",
)
```
