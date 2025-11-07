"""Implementation for `brew_formula` macro.

Select the formula file from a `brew_artifacts` declaration.
"""

def brew_formula(name, artifacts, **kwargs):
    """Selects the formula file from a `brew_artifacts` declaration.

    Args:
        name: The name of the target as a `string`.
        artifacts: A label for a `brew_artifacts` declaration.
        **kwargs: Attributes passed along to underlying rules.
    """
    native.filegroup(
        name = name,
        srcs = [artifacts],
        output_group = "formula_file",
        **kwargs
    )
