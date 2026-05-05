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

When no `.aspect/version.axl` file exists, the launcher queries the GitHub
releases API to find the latest available `aspect-cli` release and downloads
that. This is the unpinned (floating) mode — you always get the most recent
release that has a binary for your platform.

### Can you have a version.axl without pinning?

While the parser technically allows `version()` with no positional argument,
this is equivalent to not having a `version.axl` at all — the launcher will
query the releases API to find the latest available release. If you create a
`version.axl`, you should specify a version string. The only reason to have a
`version.axl` without a pinned version would be to customize the `sources`
list, e.g.:

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
| *(positional)* | No | Version string (e.g. `"2026.11.6"`). If omitted, the GitHub releases API is queried to find the latest available release. |
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
    headers = {                                                 # optional
        "Authorization": "Bearer <token>",
    },
)
```

`headers` is forwarded on the download request — useful for authenticated mirrors or private CDNs.

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
4. If the download fails, the error is reported — **no fallback to a different
   version**. When you pin, you are guaranteed to get exactly that version or
   an error.

### Unpinned version (no version.axl, or version.axl without a version string)

```
(no .aspect/version.axl file)
```

1. Check the tag hint cache — if a previous run already resolved a tag, its
   binary is cached, and the hint is less than 24 hours old, use it immediately
   with **no network call**
2. Otherwise, query the GitHub releases API
   (`/repos/{org}/{repo}/releases?per_page=10`) and scan the most recent
   non-prerelease releases to find the first one that contains the matching artifact
3. If the API call fails but a stale hint and cached binary exist, fall back to
   the cached version and reset the hint's expiry so we don't hammer a down API
4. Write the resolved tag to the hint cache for future runs
5. Direct download from
   `https://github.com/{org}/{repo}/releases/download/{resolved_tag}/{artifact}`

This means the unpinned case gets the latest *available* release on the first
run and after the 24-hour hint expiry, gracefully handles the window during a
new release where assets haven't finished uploading, avoids any network
dependency on warm-cache runs, and degrades gracefully when the GitHub API is
unavailable.

## Caching

Downloaded binaries are cached under the system cache directory
(`~/Library/Caches/aspect/launcher/downloader/` on macOS,
`~/.cache/aspect/launcher/downloader/` on Linux). The cache path is derived
from a SHA-256 hash of the tool name and source URL, so different versions
coexist without conflict.

The launcher cache root can be overridden with the `ASPECT_LAUNCHER_CACHE`
environment variable; the launcher writes its downloader cache to
`${ASPECT_LAUNCHER_CACHE}/downloader/`.

## Debugging

Set `ASPECT_DEBUG=1` to enable verbose logging of the download and caching flow.
