aspect.register_configure_extension(
    id = "bad-test",
    prepare = lambda _: aspect.PrepareResult(
        # All source files to be processed
        sources = [
            aspect.SourceExtensions(".r"),
        ],
        queries = {
            "x": aspect.RawQuery(
                f = "*.r",
            ),
        },
    ),
)
