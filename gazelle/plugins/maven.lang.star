"Maven starlark plugin"

# Directive name and default value from the rules_jvm gazelle plugin
JAVA_MAVEN_INSTALL_FILE = "java_maven_install_file"
DEFAULT_JAVA_MAVEN_INSTALL_FILE = "maven_install.json"

def prepare(ctx):
    return aspect.PrepareResult(
        # All source files to be processed
        sources = aspect.SourceFiles(ctx.properties[JAVA_MAVEN_INSTALL_FILE]),
        queries = {
            "imports": aspect.JsonQuery(
                filter = DEFAULT_JAVA_MAVEN_INSTALL_FILE,
                query = """select(.dependency_tree) | .dependency_tree.dependencies?[] | select(.packages) | {packages,coord}""",
            ),
        },
    )

def declare_maven_targets(ctx):
    if not ctx.sources:
        return

    for dep in ctx.sources[0].query_results["imports"]:
        coord = dep["coord"].rsplit(":", 1)[0].replace(".", "_").replace(":", "_")

        for pkg in dep["packages"]:
            ctx.add_symbol(
                id = pkg,
                provider_type = "java_info",
                label = aspect.Label(
                    repo = "maven",
                    name = coord,
                ),
            )

aspect.register_configure_extension(
    id = "maven",
    properties = {
        JAVA_MAVEN_INSTALL_FILE: aspect.Property(
            type = "String",
            default = DEFAULT_JAVA_MAVEN_INSTALL_FILE,
        ),
    },
    prepare = prepare,
    declare = declare_maven_targets,
)
