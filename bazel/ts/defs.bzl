"""
Defaults for Typescript projects in Silo
"""

load("@aspect_rules_swc//swc:defs.bzl", _swc = "swc")
load("@aspect_rules_ts//ts:defs.bzl", _ts_config = "ts_config", _ts_project = "ts_project")
load("@aspect_rules_ts//ts:proto.bzl", _ts_proto_library = "ts_proto_library")
load("@bazel_skylib//lib:partial.bzl", "partial")
load("@bazel_skylib//rules:write_file.bzl", _write_file = "write_file")

ts_config = _ts_config

def ts_project(name, **kwargs):
    """Macro around ts_project for silo.

    Args:
        name: Name of the ts_project target
        **kwargs: Additional attributes to pass to the ts_project rule
    """
    swcrc = ".swcrc_%s" % name
    _write_file(
        name = "swcrc_%s" % name,
        out = swcrc,
        content = json.encode({
            "inlineSourcesContent": True,
            "jsc": {
                "baseUrl": ".",
                "keepClassNames": True,
                "parser": {
                    "decorators": True,
                    "decoratorsBeforeExport": False,
                    "dynamicImport": True,
                    "syntax": "typescript",
                },
                "transform": {
                    "decoratorMetadata": True,
                    "legacyDecorator": True,
                    "react": {
                        "runtime": "automatic",
                    },
                },
            },
            "module": {
                "resolveFully": True,
                "type": "es6",
            },
            "sourceMaps": True,
        }).splitlines(),
    )

    tsconfig = kwargs.pop("tsconfig", "//bazel/ts:tsconfig.node")

    _ts_project(
        name = name,
        declaration = True,
        source_map = True,
        tsconfig = {"include": ["**/*.{ts,tsx}"]},
        extends = tsconfig,
        transpiler = partial.make(
            _swc,
            source_maps = "true",
            swcrc = swcrc,
        ),
        **kwargs
    )

def ts_proto_library(name, protoc_gen_options = {
    "js_import_style": "module",
    "target": "js+dts",
    "import_extension": ".js",
}, **kwargs):
    _ts_proto_library(
        name = name,
        protoc_gen_options = protoc_gen_options,
        proto_srcs = kwargs.pop("proto_srcs", native.glob(["*.proto"])),
        **kwargs
    )
