"""Implementation for `brew_artifacts` rule.

Generates a Homebrew formula.
"""

load("@bazel_skylib//lib:dicts.bzl", "dicts")
load("@bazel_tools//tools/build_defs/hash:hash.bzl", "sha256", sha256_tools = "tools")
load(":brews.bzl", "brews")

def _bottle_info(brew_platform, bottle_file, sha256_file, bottle_entry_file):
    return struct(
        brew_platform = brew_platform,
        bottle_file = bottle_file,
        sha256_file = sha256_file,
        bottle_entry_file = bottle_entry_file,
    )

def _bottles_info(bottle_infos, bottles_dir):
    return struct(
        bottle_infos = bottle_infos,
        bottles_dir = bottles_dir,
    )

def _process_bottle(ctx, target, brew_platform):
    tfiles = target.files.to_list()
    if len(tfiles) != 1:
        fail("A bottle target can only provide a single file. platform: ", brew_platform)
    bottle_file = tfiles[0]

    # Generate the SHA256 value
    sha256_file = sha256(ctx, bottle_file)

    bottle_entry_file = ctx.actions.declare_file(
        "{name}_{platform}.bottle_entry".format(
            name = ctx.label.name,
            platform = brew_platform,
        ),
    )
    ctx.actions.run_shell(
        outputs = [bottle_entry_file],
        inputs = [sha256_file],
        arguments = [sha256_file.path, brew_platform, bottle_entry_file.path],
        command = """\
sha256_path="$1"
platform="$2"
out_path="$3"
sha256="$(< "${sha256_path}")"
cat > "${out_path}" <<-EOF
sha256 cellar: :any_skip_relocation, ${platform}: "${sha256}"
EOF
""",
    )

    return _bottle_info(
        brew_platform = brew_platform,
        bottle_file = bottle_file,
        sha256_file = sha256_file,
        bottle_entry_file = bottle_entry_file,
    )

# Example of download URL for bottle
# http://localhost:8090/bottles/aspect-cli-0.0.0.monterey.bottle.tar.gz
#
# curl command:
# /usr/local/Homebrew/Library/Homebrew/shims/shared/curl \
#  --disable \
#  --cookie /dev/null \
#  --globoff \
#  --show-error \
#  --user-agent Homebrew/3.6.6-32-g2bec760\ \(Macintosh\;\ Intel\ Mac\ OS\ X\ 12.6\)\ curl/7.79.1 \
#  --header Accept-Language:\ en \
#  --fail \
#  --retry 3 \
#  --location \
#  --remote-time \
#  --output /Users/chuck/Library/Caches/Homebrew/downloads/1bb170bd5ac5f3aba9ca784daca78cf4897071642b70f2816d846094f30e07d4--aspect-cli-0.0.0.monterey.bottle.tar.gz.incomplete \
#  http://localhost:8090/bottles/aspect-cli-0.0.0.monterey.bottle.tar.gz

def _collect_bottles(ctx):
    bottle_infos = [
        _process_bottle(ctx, target, brew_platform)
        for (target, brew_platform) in ctx.attr.bottles.items()
    ]

    # Collect the bottles in a directory
    bottles_dir = ctx.actions.declare_directory(
        "{}_bottles".format(ctx.label.name),
    )
    args = ctx.actions.args()
    args.add(bottles_dir.path)
    args.add(ctx.attr.formula)
    args.add(ctx.file.version_file)
    inputs = [ctx.file.version_file]
    for bi in bottle_infos:
        inputs.append(bi.bottle_file)
        args.add_all([bi.brew_platform, bi.bottle_file])

    ctx.actions.run_shell(
        outputs = [bottles_dir],
        inputs = inputs,
        arguments = [args],
        command = """\
output_dir="$1"
formula="$2"
version_file="$3"
shift 3

version="$(< "${version_file}")"

# Create the output directory
mkdir -p "${output_dir}"

while (("$#")); do
    # Read args three at a time
    platform="$1"
    bottle_path="$2"
    shift 2
    
    # Copy the bottle to the output directory
    cp "${bottle_path}" "${output_dir}/${formula}-${version}.${platform}.bottle.tar.gz"
done
""",
    )

    return _bottles_info(
        bottle_infos = bottle_infos,
        bottles_dir = bottles_dir,
    )

def _ruby_class_name(ctx):
    if ctx.attr.ruby_class_name != "":
        return ctx.attr.ruby_class_name
    return brews.ruby_class_name(ctx.attr.formula)

def _write_formula(ctx, bottle_entries):
    inputs = bottle_entries[:]
    inputs.append(ctx.file.version_file)

    formula_file = ctx.actions.declare_file(
        "{}.rb".format(ctx.label.name),
    )

    args = ctx.actions.args()
    args.add("--out", formula_file)
    args.add("--ruby_class_name", _ruby_class_name(ctx))
    args.add("--desc", ctx.attr.desc)
    args.add("--homepage", ctx.attr.homepage)
    args.add("--url", ctx.attr.url)
    args.add("--version_file", ctx.file.version_file)
    if ctx.attr.license:
        args.add("--license", ctx.attr.license)
    if ctx.attr.bottle_root_url:
        args.add("--bottle_root_url", ctx.attr.bottle_root_url)
    args.add_all(bottle_entries, before_each = "--bottle_entry")

    if ctx.attr.additional_content:
        additional_content_file = ctx.actions.declare_file(
            "{}_additional_content".format(ctx.label.name),
        )
        ctx.actions.write(additional_content_file, ctx.attr.additional_content)
        args.add("--additional_content", additional_content_file)
        inputs.append(additional_content_file)

    ctx.actions.run(
        outputs = [formula_file],
        inputs = inputs,
        executable = ctx.executable._generate_formula_tool,
        arguments = [args],
    )

    return formula_file

def _brew_artifacts_impl(ctx):
    if ctx.attr.formula == "":
        fail("The formula name must not be blank.")

    bottles_info = _collect_bottles(ctx)
    bottle_entries = [bi.bottle_entry_file for bi in bottles_info.bottle_infos]
    formula_file = _write_formula(ctx, bottle_entries)

    all_files = [formula_file, bottles_info.bottles_dir]
    runfiles = ctx.runfiles(files = all_files)
    return [
        DefaultInfo(files = depset(all_files), runfiles = runfiles),
        OutputGroupInfo(
            formula_file = depset([formula_file]),
            bottle_files = depset([bottles_info.bottles_dir]),
            all_files = depset(all_files),
        ),
    ]

brew_artifacts = rule(
    implementation = _brew_artifacts_impl,
    attrs = dicts.add({
        "additional_content": attr.string(
            doc = "Additional content to add to the formula",
        ),
        "bottle_root_url": attr.string(
            doc = "The root URL from which bottles are downloaded.",
        ),
        "bottles": attr.label_keyed_string_dict(
            doc = "Maps the bottle to the Homebrew platform name.",
            allow_files = True,
        ),
        "desc": attr.string(
            mandatory = True,
            doc = "The description for the formula.",
        ),
        "formula": attr.string(
            mandatory = True,
            doc = "The name of the Homebrew formula.",
        ),
        "homepage": attr.string(
            mandatory = True,
            doc = """\
The URL to which users will be directed for information about the application.\
""",
        ),
        "license": attr.string(
            doc = "The license for formula.",
        ),
        "ruby_class_name": attr.string(
            doc = """\
The Ruby class name for the formula. If not provided, this will be calculated \
from the formula value.\
""",
        ),
        # GH329: Finish adding required formula attributes.
        # "conflicts_with": attr.string(
        #     doc = """\
        #     A JSON string representing a list of formulas that this formula conflicts with.\
        #     """,
        # ),
        # "depends_on": attr.string_list(
        #     doc = """\
        # A JSON string representing a list of formulas that this formula dependes upon.\
        # """,
        # ),
        "url": attr.string(
            doc = "The URL from which the source code can be downloaded.",
            mandatory = True,
        ),
        "version_file": attr.label(
            doc = "The file that contains the semver for the bottle.",
            mandatory = True,
            allow_single_file = True,
        ),
        "_generate_formula_tool": attr.label(
            executable = True,
            cfg = "exec",
            default = "//bazel/release/homebrew:generate_formula",
            doc = "The tool used to generate the Homebrew formula.",
        ),
    }, sha256_tools),
    doc = "Generate a Homebrew formula and collect the related bottles.",
)
