## rules_go and Gazelle

This catalogue pins `rules_go` `0.62.0` and Gazelle `0.52.2`; do not substitute an unverified newer release, and do not alter the repository's Bazel version as part of this work. If the baseline proves incompatible with the pinned Bazel, move the `rules_go` and Gazelle versions.

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

Create the root `BUILD.bazel` before evaluating these extensions. Add the root Gazelle target and prefix directive with the same Go module path. For a `go.work` workspace, first identify each independent Go module. An initial slice may model only the module that owns the selected package; record omitted modules and add a `from_file` declaration for each before claiming workspace completion. Do not point every module at the root by assumption. Run Gazelle, then `bazel mod tidy`, then build; hand-transcribing repository names from the extension's warning output is wasted work. If dependency or release-note access is unavailable, record the selected pair as unverified and stop before declaring it compatible; do not spend unbounded time researching before the local scaffold exists.

Bare `bazel run //:gazelle` walks the whole repository and writes a BUILD file for every Go package in it, which is not the initial slice. Pass the directories to generate into as positional arguments:

```text
bazel run //:gazelle -- internal/text
```

Widening that path list later is additive, and attributes Gazelle does not own survive regeneration.

Gazelle emits neither `size` nor `timeout` on generated `go_test` targets, so every test lands on the default moderate timeout. Set them where the repository's tests are known to be faster or slower than that.

A rules_go `go_test` prints `PASS` and no count in any output mode. Pass `--test_arg=-test.v` and count the `--- PASS:` lines to satisfy the validation gate.

Inventory `//go:embed` directives before generating. Gazelle maps them to `embedsrcs` on the owning `go_library`, including whole-directory and glob patterns:

```starlark
go_library(
    name = "migrator",
    srcs = ["migrator.go"],
    embedsrcs = ["migrations/0001_enums.up.sql"],
    importpath = "<module-path>/migrator",
)
```

After the first Gazelle run, check each `//go:embed` line against the generated `embedsrcs`; an embed that resolves under `go build` but is missing here fails at analysis time. Assets embedded in another language — `.sql`, `.js`, `.html` — are inputs to the Go library, not a separate build unit, and are not modelled with that language's rules.

`go_deps.from_file` makes `use_repo` a function of `go.mod` rather than of the selected slice: every direct requirement is declared, so `bazel mod tidy` writes the module's full direct-dependency set however few repositories the first target actually reaches. Do not hand-curate a shorter list expecting it to hold; `mod tidy` replaces it, and the difference between the two is not evidence that anything is wrong. The shared `.bazelrc` baseline is a completion-policy proposal: do not let it delay proving the first target, and add it before CI/remote configuration rather than copying broad policy into an exploratory slice.

```starlark
load("@gazelle//:def.bzl", "gazelle")

# gazelle:prefix <go-module-path-from-go.mod>
gazelle(
    name = "gazelle",
)
```
