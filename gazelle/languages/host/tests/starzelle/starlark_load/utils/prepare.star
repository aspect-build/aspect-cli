load("//gazelle/languages/host/tests/starzelle/starlark_load/utils:identity.star", "identity")

def _prepare(_):
    return aspect.PrepareResult(
        sources = identity([
            aspect.SourceGlobs("**/*.*"),
        ]),
    )

prepare = _prepare
