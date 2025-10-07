"""Implementation for `brew_release` macro.
"""

load("//release:platforms.bzl", "platforms")
load(":brew_artifacts.bzl", "brew_artifacts")
load(":brew_bottle.bzl", "brew_bottle")
load(":brew_platforms.bzl", "brew_platforms")
load(":brews.bzl", "brews")

def brew_go_artifacts(
        name,
        multi_platform_go_binaries,
        version_file,
        formula_name,
        desc,
        homepage,
        url,
        license,
        bottle_root_url,
        root_files = None,
        additional_content = None,
        additional_bins = [],
        **kwargs):
    """Defines targets for generating a Homebrew package with both bottles and a Homebrew formula.

    Args:
        name: The basename for the generated targets.
        multi_platform_go_binaries: The name of the multi_platform_go_binaries target.
        version_file: The file that contains the semver.
        formula_name: The name of the formula to generate
        desc: Description to set in the generated formula
        homepage: Homepage to set in the generated formula
        url: URL to set in the generated formula
        license: License to set in the generated formula
        bottle_root_url: The root_url to set in the generated formula
        root_files: Optional. The files to include at the root of the bottles.
        additional_content: Optional. Additional content to add to the generated formula.
        additional_bins: Optional. Additional bin files.
        **kwargs: Addition attributes to pass to main target.
    """

    bottles = dict()
    for bp in brew_platforms.all:
        go_binary_target_name = platforms.go_binary_target_name(multi_platform_go_binaries, bp.go_platform)
        bottle_name = brews.bottle_name(name, bp.name)
        bottles[bottle_name] = bp.name

        brew_bottle(
            name = bottle_name,
            bin_files = [go_binary_target_name] + additional_bins,
            bin_renames = {go_binary_target_name: "aspect"},
            formula = formula_name,
            root_files = root_files,
            version_file = version_file,
        )

    brew_artifacts(
        name = name,
        bottles = bottles,
        bottle_root_url = bottle_root_url,
        license = license,
        desc = desc,
        homepage = homepage,
        url = url,
        version_file = version_file,
        formula = formula_name,
        additional_content = additional_content,
        **kwargs
    )
