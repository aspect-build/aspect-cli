# Version Pinning

The `.aspect/version.axl` file pins the Aspect CLI version and configures how the launcher downloads it. Place this file in your repository root alongside `MODULE.aspect` or `MODULE.bazel`.

## Basic Usage

Pin a specific version:

```python
version("2025.46.20")
```

The launcher will download this version from the default GitHub release source.

If no `.aspect/version.axl` file exists, the launcher downloads the same version as itself.

## Custom Sources

Override where the launcher looks for the CLI binary by providing a `sources` list. Sources are tried in order; the first one that succeeds wins.

```python
version(
    "2025.46.20",
    sources = [
        local("target/debug/aspect-cli"),
        local("bazel-bin/crates/aspect-cli/aspect-cli"),
        github(
            org = "aspect-build",
            repo = "aspect-cli",
        ),
    ],
)
```

### `local(path)`

Look for a binary at a path relative to the repository root. Useful for local development.

```python
local("target/debug/aspect-cli")
```

The file is copied into the launcher cache on each invocation so that build system clean operations don't break a running CLI.

### `github(org, repo, tag?, artifact?)`

Download from a GitHub release.

```python
github(
    org = "aspect-build",
    repo = "aspect-cli",
)
```

When `tag` and `artifact` are omitted, the launcher derives defaults:
- `tag` defaults to `v{version}` (e.g. `v2025.46.20`)
- `artifact` defaults to `{repo}-{target}` (e.g. `aspect-cli-aarch64-apple-darwin`)

You can override either with explicit values:

```python
github(
    org = "aspect-build",
    repo = "aspect-cli",
    tag = "v{version}",
    artifact = "aspect-cli-{os}_{arch}",
)
```

### `http(url, headers?)`

Download from an arbitrary URL.

```python
http(
    url = "https://cdn.example.com/aspect-cli/{version}/aspect-cli-{os}-{arch}",
)
```

Pass custom headers for authenticated endpoints:

```python
http(
    url = "https://internal.example.com/aspect-cli/{version}/aspect-cli-{target}",
    headers = {
        "Authorization": "Bearer <token>",
    },
)
```

## Template Variables

String values in `tag`, `artifact`, and `url` support `{variable}` placeholders that the launcher replaces at runtime:

| Variable | Example Value | Description |
|---|---|---|
| `{version}` | `2025.46.20` | The version from the `version()` call |
| `{os}` | `darwin`, `linux` | Operating system kernel name |
| `{arch}` | `x86_64`, `aarch64` | CPU instruction set architecture |
| `{target}` | `aarch64-apple-darwin` | Full platform target triple |

These are the only supported placeholders. The file is not evaluated as Starlark; it is parsed as Starlark syntax but only string literals and function call structure are extracted.
