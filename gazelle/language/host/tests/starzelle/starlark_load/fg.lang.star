# TODO: tests should be relative to the test WORKSPACE, currently they are relative to the real WORKSPACE

load("//gazelle/language/host/tests/starzelle/starlark_load/utils:declare.star", "declare_targets")
load("//gazelle/language/host/tests/starzelle/starlark_load/utils:prepare.star", "prepare")

aspect.register_configure_extension(
    id = "fgs",
    prepare = prepare,
    declare = declare_targets,
)
