def delete_declare(ctx):
    ctx.targets.remove("deleteme")
    ctx.targets.remove("deleteme-filegroup", kind = "filegroup")
    ctx.targets.remove("deleteme-copy_to_bin", kind = "copy_to_bin")

aspect.register_configure_extension(
    id = "rm-test",
    declare = delete_declare,
)
