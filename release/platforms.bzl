"""Module for managing Go release platforms"""

_OSS = struct(
    MACOS = "darwin",
    LINUX = "linux",
    WINDOWS = "windows",
)

_ARCHS = struct(
    AMD64 = "amd64",  # x86_64
    ARM64 = "arm64",
)

_GC_LINKOPTS = struct(
    NIX = ["-s", "-w"],
    WINDOWS = [],
)

_GC_LINKOPTS_LOOKUP = {
    _OSS.MACOS: _GC_LINKOPTS.NIX,
    _OSS.LINUX: _GC_LINKOPTS.NIX,
    _OSS.WINDOWS: _GC_LINKOPTS.WINDOWS,
}

_EXTENSIONS = struct(
    NIX = "",
    WINDOWS = ".exe",
)

_EXTENSIONS_LOOKUP = {
    _OSS.MACOS: _EXTENSIONS.NIX,
    _OSS.LINUX: _EXTENSIONS.NIX,
    _OSS.WINDOWS: _EXTENSIONS.WINDOWS,
}

def _key(os, arch):
    """Create the lookup key for a Go release platform.

    Args:
        os: A `string` value from `platforms.oss`.
        arch: A `string` value from `platforms.archs`.

    Returns:
        A `string` value that can be used to uniquely identify a release
        platform.
    """
    return "{os}_{arch}".format(os = os, arch = arch)

def _new(os, arch):
    """Create a Go release platform `struct`.

    Args:
        os: A `string` value from `platforms.oss`.
        arch: A `string` value from `platforms.archs`.

    Returns:
        A `struct` representing a Go release platform.
    """
    return struct(
        os = os,
        arch = arch,
        key = _key(os, arch),
        ext = _EXTENSIONS_LOOKUP[os],
        gc_linkopts = _GC_LINKOPTS_LOOKUP[os],
    )

_PLATFORMS = [
    _new(os = _OSS.MACOS, arch = _ARCHS.AMD64),
    _new(os = _OSS.MACOS, arch = _ARCHS.ARM64),
    _new(os = _OSS.LINUX, arch = _ARCHS.AMD64),
    _new(os = _OSS.LINUX, arch = _ARCHS.ARM64),
    _new(os = _OSS.WINDOWS, arch = _ARCHS.AMD64),
]

_PLATFORMS_LOOKUP = {p.key: p for p in _PLATFORMS}

def _get(os, arch):
    """Retrieve the platform by os and arch.

    Args:
        os: A `string` value from `platforms.oss`.
        arch: A `string` value from `platforms.archs`.

    Returns:
        A `struct` representing a Go release platform as created by
        `platforms.new()`.
    """
    key = _key(os, arch)
    return _PLATFORMS_LOOKUP.get(key)

def _go_binary_target_name(basename, platform):
    """Generate a Bazel target name for a platform-specific go_binary target.

    Args:
        basename: A `string` that is used to uniquely identify the binary being
            built.
        platform: A `struct` as returned by `platforms.new()`.

    Returns:
        A `string` value that can be used as a Bazel target name.
    """
    return "{}-{}-{}".format(basename, platform.os, platform.arch)

platforms = struct(
    # Constants
    all = _PLATFORMS,
    archs = _ARCHS,
    extensions = _EXTENSIONS,
    gc_linkopts = _GC_LINKOPTS,
    oss = _OSS,
    # Functions
    get = _get,
    go_binary_target_name = _go_binary_target_name,
    key = _key,
    new = _new,
)
