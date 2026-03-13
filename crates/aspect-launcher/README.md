# aspect-launcher

The aspect-launcher is a thin bootstrap binary that provisions and executes the
full `aspect-cli`. It is distributed as the `aspect` binary that users install
(e.g. via Homebrew). When a user runs `aspect build //...`, the launcher:

1. Locates the project root (walks up from cwd looking for `MODULE.aspect`,
   `MODULE.bazel`, `WORKSPACE`, etc.)
2. Reads `.aspect/version.axl` (if present) to determine which version of
   `aspect-cli` to use and where to download it from
3. Downloads (or retrieves from cache) the correct `aspect-cli` binary
4. `exec`s the real `aspect-cli`, forwarding all arguments

The launcher also forks a child process to report anonymous usage telemetry
(honoring `DO_NOT_TRACK`).

## version.axl

The file `.aspect/version.axl` controls which `aspect-cli` version the launcher
provisions. It uses Starlark syntax and contains a single `version()` call.

### Pinned version (recommended)

```starlark
version("2026.11.6")
```

This pins the project to a specific `aspect-cli` release. The launcher downloads
directly from `https://github.com/aspect-build/aspect-cli/releases/download/v2026.11.6/<artifact>`
with no GitHub API call needed.

### Pinned version with custom sources

```starlark
version(
    "2026.11.6",
    sources = [
        local("bazel-bin/cli/aspect"),
        github(
            org = "aspect-build",
            repo = "aspect-cli",
        ),
    ],
)
```

Sources are tried in order. This example first checks for a local build, then
falls back to GitHub.

### No version.axl

When no `.aspect/version.axl` file exists, the launcher uses its own compiled-in
version and the default GitHub source. This means the `aspect-cli` version
floats with the installed launcher version.

### Can you have a version.axl without pinning?

While the parser technically allows `version()` with no positional argument
(falling back to the launcher's built-in version), this is equivalent to not
having a `version.axl` at all. If you create a `version.axl`, you should
specify a version string. The only reason to have a `version.axl` without a
pinned version would be to customize the `sources` list, e.g.:

```starlark
version(
    sources = [
        local("bazel-bin/cli/aspect"),
        github(org = "my-fork", repo = "aspect-cli"),
    ],
)
```

This is a niche use case. In general, if `version.axl` exists, pin a version.

### version() reference

```
version(<version_string>?, sources = [...]?)
```

**Arguments:**

| Argument | Required | Description |
|----------|----------|-------------|
| *(positional)* | No | Version string (e.g. `"2026.11.6"`). If omitted, defaults to the launcher's own version. |
| `sources` | No | List of source specifiers, tried in order. If omitted, defaults to `[github(org = "aspect-build", repo = "aspect-cli")]`. |

### Source types

#### github()

```starlark
github(
    org = "aspect-build",          # required
    repo = "aspect-cli",           # required
    tag = "v{version}",            # optional, default: "v{version}"
    artifact = "{repo}-{target}",  # optional, default: "{repo}-{target}"
)
```

#### http()

```starlark
http(
    url = "https://example.com/aspect-cli-{version}-{target}",  # required
)
```

#### local()

```starlark
local("bazel-bin/cli/aspect")  # path relative to project root
```

### Template variables

The `tag`, `artifact`, and `url` fields support these placeholders:

| Variable | Description | Example |
|----------|-------------|---------|
| `{version}` | The version string from `version()` | `2026.11.6` |
| `{os}` | Operating system | `darwin`, `linux` |
| `{arch}` | CPU architecture (Bazel naming) | `aarch64`, `x86_64` |
| `{target}` | LLVM target triple | `aarch64-apple-darwin`, `x86_64-unknown-linux-musl` |

## Download flow

### Pinned version (version specified in version.axl)

```
version.axl: version("2026.11.6", sources = [github(org = "aspect-build", repo = "aspect-cli")])
```

1. Tag is computed: `v2026.11.6`
2. Cache is checked — if the binary is already cached, it is used immediately
3. Direct download from
   `https://github.com/aspect-build/aspect-cli/releases/download/v2026.11.6/aspect-cli-{target}`
4. If the direct download fails, the launcher queries the GitHub releases API
   (`/repos/{org}/{repo}/releases?per_page=10`) and looks for the most recent
   release that contains the matching artifact
5. The downloaded binary is cached for future runs

### No version.axl (floating version)

```
(no .aspect/version.axl file)
```

1. Launcher uses its own compiled-in version (e.g. `2026.11.6`) and default
   GitHub source
2. Same download flow as the pinned case — the version just comes from the
   launcher binary itself instead of from a file

In both cases, the fallback to a recent release (step 4) handles the window
during a new release where the tag exists but release assets haven't finished
uploading.

## Caching

Downloaded binaries are cached under the system cache directory
(`~/Library/Caches/aspect/launcher/` on macOS, `~/.cache/aspect/launcher/` on
Linux). The cache path is derived from a SHA-256 hash of the tool name and
source URL, so different versions coexist without conflict.

The cache location can be overridden with the `ASPECT_CLI_DOWNLOADER_CACHE`
environment variable.

## Debugging

Set `ASPECT_DEBUG=1` to enable verbose logging of the download and caching flow.
