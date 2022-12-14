"macro function running bats tests using sh_test"

def bats_test(srcs = [], **kwargs):
    """macro rule running bats tests using sh_test

    Args:
        srcs: list of .bats test files,
        **kwargs: passed down to sh_test
    """
    data = kwargs.pop("data", [])
    args = kwargs.pop("args", [])
    tests = ["$(locations %s)" % src for src in srcs]
    helpers_dirs = [
        "@bats_assert//:dir",
        "@bats_support//:dir",
    ]
    env = kwargs.pop("env", {})

    env["BATS_LIB_PATH"] = ":".join(["$(locations %s)/.." % helper_dir for helper_dir in helpers_dirs])
    env["BIN"] = "$(location @bats_core//:bin)"

    native.sh_test(
        srcs = [
            "//cli/core/integration_tests:runner.sh",
        ],
        deps = [
            "@bats_core//:bats_core",
            "@bats_assert//:bats_assert",
            "@bats_support//:bats_support",
            "@bats_core//:bin",
        ] + helpers_dirs,
        env = env,
        args = tests + args,
        data = data + srcs,
        **kwargs
    )
