"""Implementation for `brews` Starlark module.

Provides functions for generating target names and attribute values for
Homebrew macros and rules.
"""

def _bottle_name(prefix_or_platform, platform = None):
    """Generate the target name for a bottle.

    Args:
        prefix_or_platform: The prefix or the platform.
        platform: If a prefix is specified, this is the platform.

    Returns:
        A `string` value.
    """
    parts = [prefix_or_platform]
    if platform != None:
        parts.append(platform)
    parts.append("bottle")
    return "_".join(parts)

def _ruby_class_name(name):
    normalized_name = name.replace("-", "_").replace(".", "_")
    parts = normalized_name.split("_")
    parts = [part.title() for part in parts]
    return "".join(parts)

brews = struct(
    bottle_name = _bottle_name,
    ruby_class_name = _ruby_class_name,
)
