"""
Defaults for Typescript projects in aspect-cli
"""

load("@aspect_rules_ts//ts:proto.bzl", _ts_proto_library = "ts_proto_library")

def ts_proto_library(name, protoc_gen_options = {
    "js_import_style": "legacy_commonjs",
    "target": "js+dts",
}, **kwargs):
    _ts_proto_library(
        name = name,
        protoc_gen_options = protoc_gen_options,
        proto_srcs = kwargs.pop("proto_srcs", native.glob(["*.proto"])),
        **kwargs
    )
