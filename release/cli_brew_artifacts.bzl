"""Implementation for `cli_brew_artifacts` macro.
"""

load("@bazel_skylib//rules:build_test.bzl", "build_test")
load("//release:platforms.bzl", "platforms")
load("//release/brew:brew_artifacts.bzl", "brew_artifacts")
load("//release/brew:brew_bottle.bzl", "brew_bottle")
load("//release/brew:brew_platforms.bzl", "brew_platforms")
load("//release/brew:brews.bzl", "brews")

def cli_brew_artifacts(
        name,
        binary_name,
        version_file,
        formula_name,
        desc,
        homepage,
        url,
        license,
        bottle_root_url,
        root_files = None,
        additional_content = None,
        additional_bins = []):
    """Defines targets for generating Homebrew bottles and a Homebrew formula.

    Args:
        name: The basename for the generated targets.
        binary_name: The basename for the binary targets.
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
    """

    bottles = dict()
    for bp in brew_platforms.all:
        go_binary_target_name = platforms.go_binary_target_name(binary_name, bp.go_platform)
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
    )

    dev_artifacts_name = "{}_dev".format(name)
    brew_artifacts(
        name = dev_artifacts_name,
        bottles = bottles,
        bottle_root_url = "http://localhost:8090/bottles",
        license = license,
        desc = desc,
        homepage = homepage,
        url = url,
        version_file = version_file,
        formula = formula_name,
        additional_content = additional_content,
    )

    build_test(
        name = "{}_build_test".format(name),
        targets = [name, dev_artifacts_name],
    )
