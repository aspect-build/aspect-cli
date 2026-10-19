# Run from a scratch workspace whose MODULE.bazel declares a repository rule
# that reads an environment variable:
#
#   env_repo = use_repo_rule("//:defs.bzl", "env_repo")
#   env_repo(name = "envrepo")
#
# and defs.bzl's rule writes `filegroup(name = "greeting_is_<$GREETING>")` into
# the repository's BUILD file (`environ = ["GREETING"]`, read with
# `rctx.getenv`). `repo_env` reaches the repository rule's environment.

def _impl(ctx: Context):
    for greeting in ["hello", "bonjour"]:
        cfg = ctx.config(repo_env = {"GREETING": greeting})
        ctx.output.print(greeting, "->", ctx.analysis().analyze("@envrepo//:all", config = cfg).keys())

main = buildx_main(impl = _impl, cli_args = {})
