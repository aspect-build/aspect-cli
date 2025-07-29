# A plugin implementing nothing
aspect.register_configure_extension(id = "nothing")

# A plugin with noop callbacks, ensuring return values are optional
aspect.register_configure_extension(
    id = "empties",
    properties = {},
    prepare = lambda _: None,
    analyze = lambda _: None,
    declare = lambda _: None,
)
