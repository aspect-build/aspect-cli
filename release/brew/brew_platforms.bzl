"""Module for managing Homebrew platforms"""

load("//release:platforms.bzl", "platforms")

# MacOS version names are listed here:
# https://github.com/Homebrew/brew/blob/74e933caa3a778213ba33ff932c7e03e8ab8b329/Library/Homebrew/macos_versions.rb#L10
_NAMES = struct(
    # homebrew CI requires arm bottles to be listed before intel bottles
    MONTEREY_X86_64 = "monterey",
    MONTEREY_ARM64 = "arm64_monterey",
    BIG_SUR_X86_64 = "big_sur",
    BIG_SUR_ARM64 = "arm64_big_sur",
    LINUX_X86_64 = "x86_64_linux",
)

def _new(name, go_platform):
    """Create a Homebrew platform `struct`.

    Args:
        name: The Homebrew name for the platforms as a `string`.
        go_platform: A `struct` representing a Go release platform as created
            by `platforms.new()`.

    Returns:
        A `struct` representing a Homebrew platform.
    """
    return struct(
        name = name,
        go_platform = go_platform,
    )

_BREW_PLATFORMS = [
    _new(_NAMES.MONTEREY_X86_64, platforms.get(platforms.oss.MACOS, platforms.archs.AMD64)),
    _new(_NAMES.MONTEREY_ARM64, platforms.get(platforms.oss.MACOS, platforms.archs.ARM64)),
    _new(_NAMES.BIG_SUR_X86_64, platforms.get(platforms.oss.MACOS, platforms.archs.AMD64)),
    _new(_NAMES.BIG_SUR_ARM64, platforms.get(platforms.oss.MACOS, platforms.archs.ARM64)),
    _new(_NAMES.LINUX_X86_64, platforms.get(platforms.oss.LINUX, platforms.archs.AMD64)),
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
