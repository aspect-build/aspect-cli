"macro function running bats tests using sh_test"

def bats_test(srcs = [], **kwargs):
    """macro rule running bats tests using sh_test

    Args:
        srcs: list of .bats test files,
        **kwargs: passed down to sh_test
    """
    data = kwargs.pop("data", [])
    args = kwargs.pop("args", [])
    tests = ["$(execpaths %s)" % src for src in srcs]
    helpers_dirs = [
        "@bats_assert//:dir",
        "@bats_support//:dir",
        "@bats_detik//:dir",
        "@bats_file//:dir",
        "@bats_mock//:dir",
    ]
    env = kwargs.pop("env", {})

    env["BATS_LIB_PATH"] = ":".join(["$(rootpaths %s)/.." % helper_dir for helper_dir in helpers_dirs])
    env["BIN"] = "$(rootpath @bats_core//:bin)"

    native.sh_test(
        srcs = [
            "//integration_tests:runner.sh",
        ],
        deps = [
            "@bats_assert//:bats_assert",
            "@bats_file//:bats_file",
            "@bats_mock//:bats_mock",
            "@bats_core//:bats_core",
            "@bats_core//:bin",
            "@bats_detik//:bats_detik",
            "@bats_support//:bats_support",
        ] + helpers_dirs,
        env = env,
        args = tests + args,
        data = data + srcs,
        **kwargs
    )
