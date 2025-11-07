"""Module for managing Homebrew platforms"""

load("//bazel/release:platforms.bzl", "platforms")

# MacOS version names are listed here:
# https://github.com/Homebrew/brew/blob/74e933caa3a778213ba33ff932c7e03e8ab8b329/Library/Homebrew/macos_versions.rb#L10
_NAMES = struct(
    # homebrew CI requires arm bottles to be listed before intel bottles
    MONTEREY_X86_64 = "monterey",
    MONTEREY_ARM64 = "arm64_monterey",
    BIG_SUR_X86_64 = "big_sur",
    BIG_SUR_ARM64 = "arm64_big_sur",
    LINUX_X86_64 = "x86_64_linux",
    LINUX_ARM64 = "arm64_linux",
)

def _new(name, rust_platform):
    """Create a Homebrew platform `struct`.

    Args:
        name: The Homebrew name for the platforms as a `string`.
        rust_platform: A `struct` representing a Go release platform as created
            by `platforms.new()`.

    Returns:
        A `struct` representing a Homebrew platform.
    """
    return struct(
        name = name,
        rust_platform = rust_platform,
    )

_BREW_PLATFORMS = [
    _new(_NAMES.MONTEREY_X86_64, platforms.get(os = platforms.oss.MACOS, arch = platforms.archs.AMD64)),
    _new(_NAMES.MONTEREY_ARM64, platforms.get(os = platforms.oss.MACOS, arch = platforms.archs.ARM64)),
    _new(_NAMES.BIG_SUR_X86_64, platforms.get(os = platforms.oss.MACOS, arch = platforms.archs.AMD64)),
    _new(_NAMES.BIG_SUR_ARM64, platforms.get(os = platforms.oss.MACOS, arch = platforms.archs.ARM64)),
    _new(_NAMES.LINUX_X86_64, platforms.get(os = platforms.oss.LINUX, arch = platforms.archs.AMD64)),
    _new(_NAMES.LINUX_ARM64, platforms.get(os = platforms.oss.LINUX, arch = platforms.archs.ARM64)),
]

_BREW_PLATFORMS_LOOKUP = {
    bp.name: bp
    for bp in _BREW_PLATFORMS
}

brew_platforms = struct(
    # Constants
    all = _BREW_PLATFORMS,
    names = _NAMES,
    by_name = _BREW_PLATFORMS_LOOKUP,
    # Functions
    new = _new,
)
