"This file managed by `bazel run //:gazelle_update_repos`"

load("@bazel_gazelle//:deps.bzl", "go_repository")

def deps():
    "Fetch go dependencies"
    go_repository(
        name = "cat_dario_mergo",
        build_file_proto_mode = "disable_global",
        importpath = "dario.cat/mergo",
        sum = "h1:Ra4+bf83h2ztPIQYNP99R6m+Y7KfnARDfID+a+vLl4s=",
        version = "v1.0.1",
    )
    go_repository(
        name = "com_github_alecthomas_assert_v2",
        build_file_proto_mode = "disable_global",
        importpath = "github.com/alecthomas/assert/v2",
        sum = "h1:2Q9r3ki8+JYXvGsDyBXwH3LcJ+WK5D0gc5E8vS6K3D0=",
        version = "v2.11.0",
    )
    go_repository(
        name = "com_github_alecthomas_chroma_v2",
        build_file_proto_mode = "disable_global",
        importpath = "github.com/alecthomas/chroma/v2",
        sum = "h1:LxXTQHFoYrstG2nnV9y2X5O94sOBzf0CIUpSTbpxvMc=",
        version = "v2.15.0",
    )
    go_repository(
        name = "com_github_alecthomas_repr",
        build_file_proto_mode = "disable_global",
        importpath = "github.com/alecthomas/repr",
        sum = "h1:GhI2A8MACjfegCPVq9f1FLvIBS+DrQ2KQBFZP1iFzXc=",
        version = "v0.4.0",
    )
    go_repository(
        name = "com_github_alphadose_haxmap",
        build_file_proto_mode = "disable_global",
        importpath = "github.com/alphadose/haxmap",
        sum = "h1:1yn+oGzy2THJj1DMuJBzRanE3sMnDAjJVbU0L31Jp3w=",
        version = "v1.4.0",
    )
    go_repository(
        name = "com_github_anmitsu_go_shlex",
        build_file_proto_mode = "disable_global",
        importpath = "github.com/anmitsu/go-shlex",
        sum = "h1:9AeTilPcZAjCFIImctFaOjnTIavg87rW78vTPkQqLI8=",
        version = "v0.0.0-20200514113438-38f4b401e2be",
    )
    go_repository(
        name = "com_github_aristanetworks_goarista",
        build_file_proto_mode = "disable_global",
        importpath = "github.com/aristanetworks/goarista",
        sum = "h1:xnu1MpGNjnEooJ8lWa3YiU3330zMZISReAosjpZpLzg=",
        version = "v0.0.0-20220328143245-64c8d3945829",
    )
    go_repository(
        name = "com_github_armon_go_metrics",
        build_file_proto_mode = "disable_global",
        importpath = "github.com/armon/go-metrics",
        sum = "h1:hR91U9KYmb6bLBYLQjyM+3j+rcd/UhE+G78SFnF8gJA=",
        version = "v0.4.1",
    )
    go_repository(
        name = "com_github_armon_go_socks5",
        build_file_proto_mode = "disable_global",
        importpath = "github.com/armon/go-socks5",
        sum = "h1:0CwZNZbxp69SHPdPJAN/hZIm0C4OItdklCFmMRWYpio=",
        version = "v0.0.0-20160902184237-e75332964ef5",
    )
    go_repository(
        name = "com_github_atotto_clipboard",
        build_file_proto_mode = "disable_global",
        importpath = "github.com/atotto/clipboard",
        sum = "h1:EH0zSVneZPSuFR11BlR9YppQTVDbh5+16AmcJi4g1z4=",
        version = "v0.1.4",
    )
    go_repository(
        name = "com_github_aymanbagabas_go_osc52_v2",
        build_file_proto_mode = "disable_global",
        importpath = "github.com/aymanbagabas/go-osc52/v2",
        sum = "h1:HwpRHbFMcZLEVr42D4p7XBqjyuxQH5SMiErDT4WkJ2k=",
        version = "v2.0.1",
    )
    go_repository(
        name = "com_github_aymanbagabas_go_udiff",
        build_file_proto_mode = "disable_global",
        importpath = "github.com/aymanbagabas/go-udiff",
        sum = "h1:TK0fH4MteXUDspT88n8CKzvK0X9O2xu9yQjWpi6yML8=",
        version = "v0.2.0",
    )
    go_repository(
        name = "com_github_aymerick_douceur",
        build_file_proto_mode = "disable_global",
        importpath = "github.com/aymerick/douceur",
        sum = "h1:Mv+mAeH1Q+n9Fr+oyamOlAkUNPWPlA8PPGR0QAaYuPk=",
        version = "v0.2.0",
    )
    go_repository(
        name = "com_github_bazel_contrib_rules_jvm",
        build_file_proto_mode = "disable_global",
        importpath = "github.com/bazel-contrib/rules_jvm",
        sum = "h1:Qkew39EgVTv+v0if0d8KZXcgazMx3mxg0LGwkcZ7Rd0=",
        version = "v0.27.0",
    )
    go_repository(
        name = "com_github_bazelbuild_bazel_gazelle",
        build_file_proto_mode = "disable_global",
        importpath = "github.com/bazelbuild/bazel-gazelle",
        sum = "h1:Us/BcN+zpIpA1yfuOCZSKhTjd+VPvDETQTVC5BwAIFE=",
        version = "v0.42.1-0.20250322095255-186298911d38",
    )
    go_repository(
        name = "com_github_bazelbuild_bazelisk",
        build_file_generation = "clean",
        build_file_proto_mode = "disable_global",
        importpath = "github.com/bazelbuild/bazelisk",
        sum = "h1:TDt+a1PYrnBF9on3WRJUisXXFhCMrhcNo8OebyS5Q34=",
        version = "v1.17.0",
    )
    go_repository(
        name = "com_github_bazelbuild_buildtools",
        build_file_proto_mode = "disable_global",
        importpath = "github.com/bazelbuild/buildtools",
        sum = "h1:FGzENZi+SX9I7h9xvMtRA3rel8hCEfyzSixteBgn7MU=",
        version = "v0.0.0-20240918101019-be1c24cc9a44",
    )
    go_repository(
        name = "com_github_bazelbuild_rules_go",
        build_file_proto_mode = "disable_global",
        importpath = "github.com/bazelbuild/rules_go",
        sum = "h1:+ozpngVAW67pCAwfhepaXSSrG3yHcj8K9hNAxSYBno4=",
        version = "v0.52.0",
    )
    go_repository(
        name = "com_github_bazelbuild_rules_python_gazelle",
        # Overwrite BUILD files in rules_python which reference Bazel symbols
        build_file_generation = "on",
        build_file_proto_mode = "disable_global",
        importpath = "github.com/bazelbuild/rules_python/gazelle",
        patch_args = ["-p2"],  # keep
        patches = ["//patches:rules_python-fcf7221c1e079307ff13d32239b7782d2f1dc48c.patch"],
        sum = "h1:OpDDYqUhemfIN9aBoTbJznXzI5pJijMiiNqaUcUKyu0=",
        version = "v0.0.0-20240701102336-084b877c98b5",
    )
    go_repository(
        name = "com_github_bgentry_go_netrc",
        build_file_proto_mode = "disable_global",
        importpath = "github.com/bgentry/go-netrc",
        sum = "h1:xDfNPAt8lFiC1UJrqV3uuy861HCTo708pDMbjHHdCas=",
        version = "v0.0.0-20140422174119-9fd32a8b3d3d",
    )
    go_repository(
        name = "com_github_bluekeyes_go_gitdiff",
        build_file_proto_mode = "disable_global",
        importpath = "github.com/bluekeyes/go-gitdiff",
        sum = "h1:SElKwtm/IQPOwKs0vdowW5uAlip+P+jatagmUU8E0r4=",
        version = "v0.7.3",
    )
    go_repository(
        name = "com_github_bmatcuk_doublestar_v4",
        build_file_proto_mode = "disable_global",
        importpath = "github.com/bmatcuk/doublestar/v4",
        sum = "h1:54Bopc5c2cAvhLRAzqOGCYHYyhcDHsFF4wWIR5wKP38=",
        version = "v4.8.1",
    )
    go_repository(
        name = "com_github_bradleyjkemp_cupaloy_v2",
        build_file_proto_mode = "disable_global",
        importpath = "github.com/bradleyjkemp/cupaloy/v2",
        sum = "h1:any4BmKE+jGIaMpnU8YgH/I2LPiLBufr6oMMlVBbn9M=",
        version = "v2.8.0",
    )
    go_repository(
        name = "com_github_bufbuild_protocompile",
        build_file_proto_mode = "disable_global",
        importpath = "github.com/bufbuild/protocompile",
        sum = "h1:LbFKd2XowZvQ/kajzguUp2DC9UEIQhIq77fZZlaQsNA=",
        version = "v0.4.0",
    )
    go_repository(
        name = "com_github_burntsushi_toml",
        build_file_proto_mode = "disable_global",
        importpath = "github.com/BurntSushi/toml",
        sum = "h1:kuoIxZQy2WRRk1pttg9asf+WVv6tWQuBNVmK8+nqPr0=",
        version = "v1.4.0",
    )
    go_repository(
        name = "com_github_bwesterb_go_ristretto",
        build_file_proto_mode = "disable_global",
        importpath = "github.com/bwesterb/go-ristretto",
        sum = "h1:1w53tCkGhCQ5djbat3+MH0BAQ5Kfgbt56UZQ/JMzngw=",
        version = "v1.2.3",
    )
    go_repository(
        name = "com_github_catppuccin_go",
        build_file_proto_mode = "disable_global",
        importpath = "github.com/catppuccin/go",
        sum = "h1:d+0/YicIq+hSTo5oPuRi5kOpqkVA5tAsU6dNhvRu+aY=",
        version = "v0.3.0",
    )
    go_repository(
        name = "com_github_census_instrumentation_opencensus_proto",
        build_file_proto_mode = "disable_global",
        importpath = "github.com/census-instrumentation/opencensus-proto",
        sum = "h1:iKLQ0xPNFxR/2hzXZMrBo8f1j86j5WHzznCCQxV/b8g=",
        version = "v0.4.1",
    )
    go_repository(
        name = "com_github_cespare_xxhash_v2",
        build_file_proto_mode = "disable_global",
        importpath = "github.com/cespare/xxhash/v2",
        sum = "h1:DC2CZ1Ep5Y4k3ZQ899DldepgrayRUGE6BBZ/cd9Cj44=",
        version = "v2.2.0",
    )
    go_repository(
        name = "com_github_charmbracelet_bubbles",
        build_file_proto_mode = "disable_global",
        importpath = "github.com/charmbracelet/bubbles",
        sum = "h1:jSZu6qD8cRQ6k9OMfR1WlM+ruM8fkPWkHvQWD9LIutE=",
        version = "v0.20.0",
    )
    go_repository(
        name = "com_github_charmbracelet_bubbletea",
        build_file_proto_mode = "disable_global",
        importpath = "github.com/charmbracelet/bubbletea",
        sum = "h1:WpU6fCY0J2vDWM3zfS3vIDi/ULq3SYphZhkAGGvmEUY=",
        version = "v1.3.3",
    )
    go_repository(
        name = "com_github_charmbracelet_glamour",
        build_file_proto_mode = "disable_global",
        importpath = "github.com/charmbracelet/glamour",
        sum = "h1:tPrjL3aRcQbn++7t18wOpgLyl8wrOHUEDS7IZ68QtZs=",
        version = "v0.8.0",
    )
    go_repository(
        name = "com_github_charmbracelet_harmonica",
        build_file_proto_mode = "disable_global",
        importpath = "github.com/charmbracelet/harmonica",
        sum = "h1:8NxJWRWg/bzKqqEaaeFNipOu77YR5t8aSwG4pgaUBiQ=",
        version = "v0.2.0",
    )
    go_repository(
        name = "com_github_charmbracelet_huh",
        build_file_proto_mode = "disable_global",
        importpath = "github.com/charmbracelet/huh",
        sum = "h1:mZM8VvZGuE0hoDXq6XLxRtgfWyTI3b2jZNKh0xWmax8=",
        version = "v0.6.0",
    )
    go_repository(
        name = "com_github_charmbracelet_huh_spinner",
        build_file_proto_mode = "disable_global",
        importpath = "github.com/charmbracelet/huh/spinner",
        sum = "h1:ema1/VJc+iRkxKd78YdBkc9FKW9UY90ZYvfci2HIOUI=",
        version = "v0.0.0-20250213143221-71c9d72e6770",
    )
    go_repository(
        name = "com_github_charmbracelet_lipgloss",
        build_file_proto_mode = "disable_global",
        importpath = "github.com/charmbracelet/lipgloss",
        sum = "h1:O7VkGDvqEdGi93X+DeqsQ7PKHDgtQfF8j8/O2qFMQNg=",
        version = "v1.0.0",
    )
    go_repository(
        name = "com_github_charmbracelet_x_ansi",
        build_file_proto_mode = "disable_global",
        importpath = "github.com/charmbracelet/x/ansi",
        sum = "h1:9GTq3xq9caJW8ZrBTe0LIe2fvfLR/bYXKTx2llXn7xE=",
        version = "v0.8.0",
    )
    go_repository(
        name = "com_github_charmbracelet_x_exp_golden",
        build_file_proto_mode = "disable_global",
        importpath = "github.com/charmbracelet/x/exp/golden",
        sum = "h1:MnAMdlwSltxJyULnrYbkZpp4k58Co7Tah3ciKhSNo0Q=",
        version = "v0.0.0-20240815200342-61de596daa2b",
    )
    go_repository(
        name = "com_github_charmbracelet_x_exp_strings",
        build_file_proto_mode = "disable_global",
        importpath = "github.com/charmbracelet/x/exp/strings",
        sum = "h1:k2jFXp3mIsJ1lqGzpABadj9sGInRyk7kTxXfM/Lo1d0=",
        version = "v0.0.0-20250213125511-a0c32e22e4fc",
    )
    go_repository(
        name = "com_github_charmbracelet_x_term",
        build_file_proto_mode = "disable_global",
        importpath = "github.com/charmbracelet/x/term",
        sum = "h1:AQeHeLZ1OqSXhrAWpYUtZyX1T3zVxfpZuEQMIQaGIAQ=",
        version = "v0.2.1",
    )
    go_repository(
        name = "com_github_chzyer_logex",
        build_file_proto_mode = "disable_global",
        importpath = "github.com/chzyer/logex",
        sum = "h1:XHDu3E6q+gdHgsdTPH6ImJMIp436vR6MPtH8gP05QzM=",
        version = "v1.2.1",
    )
    go_repository(
        name = "com_github_chzyer_readline",
        build_file_proto_mode = "disable_global",
        importpath = "github.com/chzyer/readline",
        sum = "h1:upd/6fQk4src78LMRzh5vItIt361/o4uq553V8B5sGI=",
        version = "v1.5.1",
    )
    go_repository(
        name = "com_github_chzyer_test",
        build_file_proto_mode = "disable_global",
        importpath = "github.com/chzyer/test",
        sum = "h1:p3BQDXSxOhOG0P9z6/hGnII4LGiEPOYBhs8asl/fC04=",
        version = "v1.0.0",
    )
    go_repository(
        name = "com_github_cloudflare_circl",
        build_file_proto_mode = "disable_global",
        importpath = "github.com/cloudflare/circl",
        # See https://github.com/bazelbuild/bazel-gazelle/issues/1421
        patches = [
            "//:patches/com_github_cloudflare_circl/fp25519.patch",
            "//:patches/com_github_cloudflare_circl/fp448.patch",
            "//:patches/com_github_cloudflare_circl/x25519.patch",
            "//:patches/com_github_cloudflare_circl/x448.patch",
        ],
        sum = "h1:cr5JKic4HI+LkINy2lg3W2jF8sHCVTBncJr5gIIq7qk=",
        version = "v1.6.0",
    )
    go_repository(
        name = "com_github_cncf_xds_go",
        build_file_proto_mode = "disable_global",
        importpath = "github.com/cncf/xds/go",
        sum = "h1:DBmgJDC9dTfkVyGgipamEh2BpGYxScCH1TOF1LL1cXc=",
        version = "v0.0.0-20240318125728-8a4994d93e50",
    )
    go_repository(
        name = "com_github_coreos_go_semver",
        build_file_proto_mode = "disable_global",
        importpath = "github.com/coreos/go-semver",
        sum = "h1:wkHLiw0WNATZnSG7epLsujiMCgPAc9xhjJ4tgnAxmfM=",
        version = "v0.3.0",
    )
    go_repository(
        name = "com_github_coreos_go_systemd_v22",
        build_file_proto_mode = "disable_global",
        importpath = "github.com/coreos/go-systemd/v22",
        sum = "h1:RrqgGjYQKalulkV8NGVIfkXQf6YYmOyiJKk8iXXhfZs=",
        version = "v22.5.0",
    )
    go_repository(
        name = "com_github_cpuguy83_go_md2man_v2",
        build_file_proto_mode = "disable_global",
        importpath = "github.com/cpuguy83/go-md2man/v2",
        sum = "h1:XJtiaUW6dEEqVuZiMTn1ldk455QWwEIsMIJlo5vtkx0=",
        version = "v2.0.6",
    )
    go_repository(
        name = "com_github_creack_pty",
        build_file_proto_mode = "disable_global",
        importpath = "github.com/creack/pty",
        sum = "h1:bJrF4RRfyJnbTJqzRLHzcGaZK1NeM5kTC9jGgovnR1s=",
        version = "v1.1.24",
    )
    go_repository(
        name = "com_github_cyphar_filepath_securejoin",
        build_file_proto_mode = "disable_global",
        importpath = "github.com/cyphar/filepath-securejoin",
        sum = "h1:JyxxyPEaktOD+GAnqIqTf9A8tHyAG22rowi7HkoSU1s=",
        version = "v0.4.1",
    )
    go_repository(
        name = "com_github_davecgh_go_spew",
        build_file_proto_mode = "disable_global",
        importpath = "github.com/davecgh/go-spew",
        sum = "h1:U9qPSI2PIWSS1VwoXQT9A3Wy9MM3WgvqSxFWenqJduM=",
        version = "v1.1.2-0.20180830191138-d8f796af33cc",
    )
    go_repository(
        name = "com_github_dlclark_regexp2",
        build_file_proto_mode = "disable_global",
        importpath = "github.com/dlclark/regexp2",
        sum = "h1:Q/sSnsKerHeCkc/jSTNq1oCm7KiVgUMZRDUoRu0JQZQ=",
        version = "v1.11.5",
    )
    go_repository(
        name = "com_github_dustin_go_humanize",
        build_file_proto_mode = "disable_global",
        importpath = "github.com/dustin/go-humanize",
        sum = "h1:GzkhY7T5VNhEkwH0PVJgjz+fX1rhBrR7pRT3mDkpeCY=",
        version = "v1.0.1",
    )
    go_repository(
        name = "com_github_elazarl_goproxy",
        build_file_proto_mode = "disable_global",
        importpath = "github.com/elazarl/goproxy",
        sum = "h1:4GyuSbFa+s26+3rmYNSuUVsx+HgPrV1bk1jXI0l9wjM=",
        version = "v1.4.0",
    )
    go_repository(
        name = "com_github_emirpasic_gods",
        build_file_proto_mode = "disable_global",
        importpath = "github.com/emirpasic/gods",
        sum = "h1:FXtiHYKDGKCW2KzwZKx0iC0PQmdlorYgdFG9jPXJ1Bc=",
        version = "v1.18.1",
    )
    go_repository(
        name = "com_github_envoyproxy_go_control_plane",
        build_file_proto_mode = "disable_global",
        importpath = "github.com/envoyproxy/go-control-plane",
        sum = "h1:4X+VP1GHd1Mhj6IB5mMeGbLCleqxjletLK6K0rbxyZI=",
        version = "v0.12.0",
    )
    go_repository(
        name = "com_github_envoyproxy_protoc_gen_validate",
        build_file_proto_mode = "disable_global",
        importpath = "github.com/envoyproxy/protoc-gen-validate",
        sum = "h1:gVPz/FMfvh57HdSJQyvBtF00j8JU4zdyUgIUNhlgg0A=",
        version = "v1.0.4",
    )
    go_repository(
        name = "com_github_erikgeiser_coninput",
        build_file_proto_mode = "disable_global",
        importpath = "github.com/erikgeiser/coninput",
        sum = "h1:Y/CXytFA4m6baUTXGLOoWe4PQhGxaX0KpnayAqC48p4=",
        version = "v0.0.0-20211004153227-1c3628e74d0f",
    )
    go_repository(
        name = "com_github_fatih_color",
        build_file_proto_mode = "disable_global",
        importpath = "github.com/fatih/color",
        sum = "h1:S8gINlzdQ840/4pfAwic/ZE0djQEH3wM94VfqLTZcOM=",
        version = "v1.18.0",
    )
    go_repository(
        name = "com_github_felixge_httpsnoop",
        build_file_proto_mode = "disable_global",
        importpath = "github.com/felixge/httpsnoop",
        sum = "h1:NFTV2Zj1bL4mc9sqWACXbQFVBBg2W3GPvqp8/ESS2Wg=",
        version = "v1.0.4",
    )
    go_repository(
        name = "com_github_frankban_quicktest",
        build_file_proto_mode = "disable_global",
        importpath = "github.com/frankban/quicktest",
        sum = "h1:7Xjx+VpznH+oBnejlPUj8oUpdxnVs4f8XU8WnHkI4W8=",
        version = "v1.14.6",
    )
    go_repository(
        name = "com_github_fsnotify_fsnotify",
        build_file_proto_mode = "disable_global",
        importpath = "github.com/fsnotify/fsnotify",
        sum = "h1:dAwr6QBTBZIkG8roQaJjGof0pp0EeF+tNV7YBP3F/8M=",
        version = "v1.8.0",
    )
    go_repository(
        name = "com_github_gertd_go_pluralize",
        build_file_proto_mode = "disable_global",
        importpath = "github.com/gertd/go-pluralize",
        sum = "h1:M3uASbVjMnTsPb0PNqg+E/24Vwigyo/tvyMTtAlLgiA=",
        version = "v0.2.1",
    )
    go_repository(
        name = "com_github_ghodss_yaml",
        build_file_proto_mode = "disable_global",
        importpath = "github.com/ghodss/yaml",
        sum = "h1:wQHKEahhL6wmXdzwWG11gIVCkOv05bNOh+Rxn0yngAk=",
        version = "v1.0.0",
    )
    go_repository(
        name = "com_github_gliderlabs_ssh",
        build_file_proto_mode = "disable_global",
        importpath = "github.com/gliderlabs/ssh",
        sum = "h1:a4YXD1V7xMF9g5nTkdfnja3Sxy1PVDCj1Zg4Wb8vY6c=",
        version = "v0.3.8",
    )
    go_repository(
        name = "com_github_go_git_gcfg",
        build_file_proto_mode = "disable_global",
        importpath = "github.com/go-git/gcfg",
        sum = "h1:+zs/tPmkDkHx3U66DAb0lQFJrpS6731Oaa12ikc+DiI=",
        version = "v1.5.1-0.20230307220236-3a3c6141e376",
    )
    go_repository(
        name = "com_github_go_git_go_billy_v5",
        build_file_proto_mode = "disable_global",
        importpath = "github.com/go-git/go-billy/v5",
        sum = "h1:6Q86EsPXMa7c3YZ3aLAQsMA0VlWmy43r6FHqa/UNbRM=",
        version = "v5.6.2",
    )
    go_repository(
        name = "com_github_go_git_go_git_fixtures_v4",
        build_file_proto_mode = "disable_global",
        importpath = "github.com/go-git/go-git-fixtures/v4",
        sum = "h1:eMje31YglSBqCdIqdhKBW8lokaMrL3uTkpGYlE2OOT4=",
        version = "v4.3.2-0.20231010084843-55a94097c399",
    )
    go_repository(
        name = "com_github_go_git_go_git_v5",
        build_file_proto_mode = "disable_global",
        importpath = "github.com/go-git/go-git/v5",
        sum = "h1:7O7xvsK7K+rZPKW6AQR1YyNhfywkv7B8/FsP3ki6Zv0=",
        version = "v5.13.2",
    )
    go_repository(
        name = "com_github_go_logr_logr",
        build_file_proto_mode = "disable_global",
        importpath = "github.com/go-logr/logr",
        sum = "h1:pKouT5E8xu9zeFC39JXRDukb6JFQPXM5p5I91188VAQ=",
        version = "v1.4.1",
    )
    go_repository(
        name = "com_github_go_logr_stdr",
        build_file_proto_mode = "disable_global",
        importpath = "github.com/go-logr/stdr",
        sum = "h1:hSWxHoqTgW2S2qGc0LTAI563KZ5YKYRhT3MFKZMbjag=",
        version = "v1.2.2",
    )
    go_repository(
        name = "com_github_go_sprout_sprout",
        build_file_proto_mode = "disable_global",
        importpath = "github.com/go-sprout/sprout",
        sum = "h1:4uxG1fZbUxfXB2OsjwzyIOK4lZQEhsksib17vVAZqOs=",
        version = "v1.0.0",
    )
    go_repository(
        name = "com_github_go_task_slim_sprig_v3",
        build_file_proto_mode = "disable_global",
        importpath = "github.com/go-task/slim-sprig/v3",
        sum = "h1:sUs3vkvUymDpBKi3qH1YSqBQk9+9D/8M2mN1vB6EwHI=",
        version = "v3.0.0",
    )
    go_repository(
        name = "com_github_godbus_dbus_v5",
        build_file_proto_mode = "disable_global",
        importpath = "github.com/godbus/dbus/v5",
        sum = "h1:9349emZab16e7zQvpmsbtjc18ykshndd8y2PG3sgJbA=",
        version = "v5.0.4",
    )
    go_repository(
        name = "com_github_gogo_protobuf",
        build_file_proto_mode = "disable_global",
        importpath = "github.com/gogo/protobuf",
        sum = "h1:Ov1cvc58UF3b5XjBnZv7+opcTcQFZebYjWzi34vdm4Q=",
        version = "v1.3.2",
    )
    go_repository(
        name = "com_github_golang_glog",
        build_file_proto_mode = "disable_global",
        importpath = "github.com/golang/glog",
        sum = "h1:uCdmnmatrKCgMBlM4rMuJZWOkPDqdbZPnrMXDY4gI68=",
        version = "v1.2.0",
    )
    go_repository(
        name = "com_github_golang_groupcache",
        build_file_proto_mode = "disable_global",
        importpath = "github.com/golang/groupcache",
        sum = "h1:f+oWsMOmNPc8JmEHVZIycC7hBoQxHH9pNKQORJNozsQ=",
        version = "v0.0.0-20241129210726-2c02b8208cf8",
    )
    go_repository(
        name = "com_github_golang_mock",
        build_file_proto_mode = "disable_global",
        importpath = "github.com/golang/mock",
        sum = "h1:YojYx61/OLFsiv6Rw1Z96LpldJIy31o+UHmwAUMJ6/U=",
        version = "v1.7.0-rc.1",
    )
    go_repository(
        name = "com_github_golang_protobuf",
        build_file_proto_mode = "disable_global",
        importpath = "github.com/golang/protobuf",
        sum = "h1:i7eJL8qZTpSEXOPTxNKhASYpMn+8e5Q6AdndVa1dWek=",
        version = "v1.5.4",
    )
    go_repository(
        name = "com_github_google_btree",
        build_file_proto_mode = "disable_global",
        importpath = "github.com/google/btree",
        sum = "h1:xf4v41cLI2Z6FxbKm+8Bu+m8ifhj15JuZ9sa0jZCMUU=",
        version = "v1.1.2",
    )
    go_repository(
        name = "com_github_google_go_cmp",
        build_file_proto_mode = "disable_global",
        importpath = "github.com/google/go-cmp",
        sum = "h1:ofyhxvXcZhMsU5ulbFiLKl/XBFqE1GSq7atu8tAmTRI=",
        version = "v0.6.0",
    )
    go_repository(
        name = "com_github_google_pprof",
        build_file_proto_mode = "disable_global",
        importpath = "github.com/google/pprof",
        sum = "h1:k7nVchz72niMH6YLQNvHSdIE7iqsQxK1P41mySCvssg=",
        version = "v0.0.0-20240424215950-a892ee059fd6",
    )
    go_repository(
        name = "com_github_google_s2a_go",
        build_file_proto_mode = "disable_global",
        importpath = "github.com/google/s2a-go",
        sum = "h1:60BLSyTrOV4/haCDW4zb1guZItoSq8foHCXrAnjBo/o=",
        version = "v0.1.7",
    )
    go_repository(
        name = "com_github_google_uuid",
        build_file_proto_mode = "disable_global",
        importpath = "github.com/google/uuid",
        sum = "h1:NIvaJDMOsjHA8n1jAhLSgzrAzy1Hgr+hNrb57e+94F0=",
        version = "v1.6.0",
    )
    go_repository(
        name = "com_github_googleapis_enterprise_certificate_proxy",
        build_file_proto_mode = "disable_global",
        importpath = "github.com/googleapis/enterprise-certificate-proxy",
        sum = "h1:Vie5ybvEvT75RniqhfFxPRy3Bf7vr3h0cechB90XaQs=",
        version = "v0.3.2",
    )
    go_repository(
        name = "com_github_googleapis_gax_go_v2",
        build_file_proto_mode = "disable_global",
        importpath = "github.com/googleapis/gax-go/v2",
        sum = "h1:5/zPPDvw8Q1SuXjrqrZslrqT7dL/uJT2CQii/cLCKqA=",
        version = "v2.12.3",
    )
    go_repository(
        name = "com_github_googleapis_google_cloud_go_testing",
        build_file_proto_mode = "disable_global",
        importpath = "github.com/googleapis/google-cloud-go-testing",
        sum = "h1:zC34cGQu69FG7qzJ3WiKW244WfhDC3xxYMeNOX2gtUQ=",
        version = "v0.0.0-20210719221736-1c9a4c676720",
    )
    go_repository(
        name = "com_github_gorilla_css",
        build_file_proto_mode = "disable_global",
        importpath = "github.com/gorilla/css",
        sum = "h1:ntNaBIghp6JmvWnxbZKANoLyuXTPZ4cAMlo6RyhlbO8=",
        version = "v1.0.1",
    )
    go_repository(
        name = "com_github_hashicorp_consul_api",
        build_file_proto_mode = "disable_global",
        importpath = "github.com/hashicorp/consul/api",
        sum = "h1:mXfkRHrpHN4YY3RqL09nXU1eHKLNiuAN4kHvDQ16k/8=",
        version = "v1.28.2",
    )
    go_repository(
        name = "com_github_hashicorp_errwrap",
        build_file_proto_mode = "disable_global",
        importpath = "github.com/hashicorp/errwrap",
        sum = "h1:OxrOeh75EUXMY8TBjag2fzXGZ40LB6IKw45YeGUDY2I=",
        version = "v1.1.0",
    )
    go_repository(
        name = "com_github_hashicorp_go_cleanhttp",
        build_file_proto_mode = "disable_global",
        importpath = "github.com/hashicorp/go-cleanhttp",
        sum = "h1:035FKYIWjmULyFRBKPs8TBQoi0x6d9G4xc9neXJWAZQ=",
        version = "v0.5.2",
    )
    go_repository(
        name = "com_github_hashicorp_go_hclog",
        build_file_proto_mode = "disable_global",
        importpath = "github.com/hashicorp/go-hclog",
        sum = "h1:Qr2kF+eVWjTiYmU7Y31tYlP1h0q/X3Nl3tPGdaB11/k=",
        version = "v1.6.3",
    )
    go_repository(
        name = "com_github_hashicorp_go_immutable_radix",
        build_file_proto_mode = "disable_global",
        importpath = "github.com/hashicorp/go-immutable-radix",
        sum = "h1:DKHmCUm2hRBK510BaiZlwvpD40f8bJFeZnpfm2KLowc=",
        version = "v1.3.1",
    )
    go_repository(
        name = "com_github_hashicorp_go_multierror",
        build_file_proto_mode = "disable_global",
        importpath = "github.com/hashicorp/go-multierror",
        sum = "h1:H5DkEtf6CXdFp0N0Em5UCwQpXMWke8IA0+lD48awMYo=",
        version = "v1.1.1",
    )
    go_repository(
        name = "com_github_hashicorp_go_plugin",
        build_file_proto_mode = "disable_global",
        importpath = "github.com/hashicorp/go-plugin",
        sum = "h1:P7MR2UP6gNKGPp+y7EZw2kOiq4IR9WiqLvp0XOsVdwI=",
        version = "v1.6.1",
    )
    go_repository(
        name = "com_github_hashicorp_go_rootcerts",
        build_file_proto_mode = "disable_global",
        importpath = "github.com/hashicorp/go-rootcerts",
        sum = "h1:jzhAVGtqPKbwpyCPELlgNWhE1znq+qwJtW5Oi2viEzc=",
        version = "v1.0.2",
    )
    go_repository(
        name = "com_github_hashicorp_go_version",
        build_file_proto_mode = "disable_global",
        importpath = "github.com/hashicorp/go-version",
        sum = "h1:5tqGy27NaOTB8yJKUZELlFAS/LTKJkrmONwQKeRZfjY=",
        version = "v1.7.0",
    )
    go_repository(
        name = "com_github_hashicorp_golang_lru",
        build_file_proto_mode = "disable_global",
        importpath = "github.com/hashicorp/golang-lru",
        sum = "h1:YDjusn29QI/Das2iO9M0BHnIbxPeyuCHsjMW+lJfyTc=",
        version = "v0.5.4",
    )
    go_repository(
        name = "com_github_hashicorp_hcl",
        build_file_proto_mode = "disable_global",
        importpath = "github.com/hashicorp/hcl",
        sum = "h1:0Anlzjpi4vEasTeNFn2mLJgTSwt0+6sfsiTG8qcWGx4=",
        version = "v1.0.0",
    )
    go_repository(
        name = "com_github_hashicorp_serf",
        build_file_proto_mode = "disable_global",
        importpath = "github.com/hashicorp/serf",
        sum = "h1:Z1H2J60yRKvfDYAOZLd2MU0ND4AH/WDz7xYHDWQsIPY=",
        version = "v0.10.1",
    )
    go_repository(
        name = "com_github_hashicorp_yamux",
        build_file_proto_mode = "disable_global",
        importpath = "github.com/hashicorp/yamux",
        sum = "h1:yrQxtgseBDrq9Y652vSRDvsKCJKOUD+GzTS4Y0Y8pvE=",
        version = "v0.1.1",
    )
    go_repository(
        name = "com_github_hay_kot_scaffold",
        build_file_proto_mode = "disable_global",
        importpath = "github.com/hay-kot/scaffold",
        sum = "h1:1eUQiciRz9fazhI48SBAGmGJ5XIqU8Og1Ygxv3z35g4=",
        version = "v0.6.1",
    )
    go_repository(
        name = "com_github_hexops_gotextdiff",
        build_file_proto_mode = "disable_global",
        importpath = "github.com/hexops/gotextdiff",
        sum = "h1:gitA9+qJrrTCsiCl7+kh75nPqQt1cx4ZkudSTLoUqJM=",
        version = "v1.0.3",
    )
    go_repository(
        name = "com_github_huandu_xstrings",
        build_file_proto_mode = "disable_global",
        importpath = "github.com/huandu/xstrings",
        sum = "h1:2ag3IFq9ZDANvthTwTiqSSZLjDc+BedvHPAp5tJy2TI=",
        version = "v1.5.0",
    )
    go_repository(
        name = "com_github_inconshreveable_mousetrap",
        build_file_proto_mode = "disable_global",
        importpath = "github.com/inconshreveable/mousetrap",
        sum = "h1:wN+x4NVGpMsO7ErUn/mUI3vEoE6Jt13X2s0bqwp9tc8=",
        version = "v1.1.0",
    )
    go_repository(
        name = "com_github_itchyny_gojq",
        build_file_proto_mode = "disable_global",
        importpath = "github.com/itchyny/gojq",
        sum = "h1:yLfgLxhIr/6sJNVmYfQjTIv0jGctu6/DgDoivmxTr7g=",
        version = "v0.12.16",
    )
    go_repository(
        name = "com_github_itchyny_timefmt_go",
        build_file_proto_mode = "disable_global",
        importpath = "github.com/itchyny/timefmt-go",
        sum = "h1:ia3s54iciXDdzWzwaVKXZPbiXzxxnv1SPGFfM/myJ5Q=",
        version = "v0.1.6",
    )
    go_repository(
        name = "com_github_jbenet_go_context",
        build_file_proto_mode = "disable_global",
        importpath = "github.com/jbenet/go-context",
        sum = "h1:BQSFePA1RWJOlocH6Fxy8MmwDt+yVQYULKfN0RoTN8A=",
        version = "v0.0.0-20150711004518-d14ea06fba99",
    )
    go_repository(
        name = "com_github_jhump_protoreflect",
        build_file_proto_mode = "disable_global",
        importpath = "github.com/jhump/protoreflect",
        sum = "h1:HUMERORf3I3ZdX05WaQ6MIpd/NJ434hTp5YiKgfCL6c=",
        version = "v1.15.1",
    )
    go_repository(
        name = "com_github_json_iterator_go",
        build_file_proto_mode = "disable_global",
        importpath = "github.com/json-iterator/go",
        sum = "h1:PV8peI4a0ysnczrg+LtxykD8LfKY9ML6u2jnxaEnrnM=",
        version = "v1.1.12",
    )
    go_repository(
        name = "com_github_kevinburke_ssh_config",
        build_file_proto_mode = "disable_global",
        importpath = "github.com/kevinburke/ssh_config",
        sum = "h1:x584FjTGwHzMwvHx18PXxbBVzfnxogHaAReU4gf13a4=",
        version = "v1.2.0",
    )
    go_repository(
        name = "com_github_klauspost_compress",
        build_file_proto_mode = "disable_global",
        importpath = "github.com/klauspost/compress",
        sum = "h1:RlWWUY/Dr4fL8qk9YG7DTZ7PDgME2V4csBXA8L/ixi4=",
        version = "v1.17.2",
    )
    go_repository(
        name = "com_github_kr_fs",
        build_file_proto_mode = "disable_global",
        importpath = "github.com/kr/fs",
        sum = "h1:Jskdu9ieNAYnjxsi0LbQp1ulIKZV1LAFgK1tWhpZgl8=",
        version = "v0.1.0",
    )
    go_repository(
        name = "com_github_kr_pretty",
        build_file_proto_mode = "disable_global",
        importpath = "github.com/kr/pretty",
        sum = "h1:flRD4NNwYAUpkphVc1HcthR4KEIFJ65n8Mw5qdRn3LE=",
        version = "v0.3.1",
    )
    go_repository(
        name = "com_github_kr_pty",
        build_file_proto_mode = "disable_global",
        importpath = "github.com/kr/pty",
        sum = "h1:VkoXIwSboBpnk99O/KFauAEILuNHv5DVFKZMBN/gUgw=",
        version = "v1.1.1",
    )
    go_repository(
        name = "com_github_kr_text",
        build_file_proto_mode = "disable_global",
        importpath = "github.com/kr/text",
        sum = "h1:5Nx0Ya0ZqY2ygV366QzturHI13Jq95ApcVaJBhpS+AY=",
        version = "v0.2.0",
    )
    go_repository(
        name = "com_github_kylelemons_godebug",
        build_file_proto_mode = "disable_global",
        importpath = "github.com/kylelemons/godebug",
        sum = "h1:RPNrshWIDI6G2gRW9EHilWtl7Z6Sb1BR0xunSBf0SNc=",
        version = "v1.1.0",
    )
    go_repository(
        name = "com_github_lucasb_eyer_go_colorful",
        build_file_proto_mode = "disable_global",
        importpath = "github.com/lucasb-eyer/go-colorful",
        sum = "h1:1nnpGOrhyZZuNyfu1QjKiUICQ74+3FNCN69Aj6K7nkY=",
        version = "v1.2.0",
    )
    go_repository(
        name = "com_github_magiconair_properties",
        build_file_proto_mode = "disable_global",
        importpath = "github.com/magiconair/properties",
        sum = "h1:IeQXZAiQcpL9mgcAe1Nu6cX9LLw6ExEHKjN0VQdvPDY=",
        version = "v1.8.7",
    )
    go_repository(
        name = "com_github_makenowjust_heredoc",
        build_file_proto_mode = "disable_global",
        importpath = "github.com/MakeNowJust/heredoc",
        sum = "h1:cXCdzVdstXyiTqTvfqk9SDHpKNjxuom+DOlyEeQ4pzQ=",
        version = "v1.0.0",
    )
    go_repository(
        name = "com_github_manifoldco_promptui",
        build_file_proto_mode = "disable_global",
        importpath = "github.com/manifoldco/promptui",
        sum = "h1:3V4HzJk1TtXW1MTZMP7mdlwbBpIinw3HztaIlYthEiA=",
        version = "v0.9.0",
    )
    go_repository(
        name = "com_github_masterminds_semver_v3",
        build_file_proto_mode = "disable_global",
        importpath = "github.com/Masterminds/semver/v3",
        sum = "h1:QtNSWtVZ3nBfk8mAOu/B6v7FMJ+NHTIgUPi7rj+4nv4=",
        version = "v3.3.1",
    )
    go_repository(
        name = "com_github_mattn_go_colorable",
        build_file_proto_mode = "disable_global",
        importpath = "github.com/mattn/go-colorable",
        sum = "h1:9A9LHSqF/7dyVVX6g0U9cwm9pG3kP9gSzcuIPHPsaIE=",
        version = "v0.1.14",
    )
    go_repository(
        name = "com_github_mattn_go_isatty",
        build_file_proto_mode = "disable_global",
        importpath = "github.com/mattn/go-isatty",
        sum = "h1:xfD0iDuEKnDkl03q4limB+vH+GxLEtL/jb4xVJSWWEY=",
        version = "v0.0.20",
    )
    go_repository(
        name = "com_github_mattn_go_localereader",
        build_file_proto_mode = "disable_global",
        importpath = "github.com/mattn/go-localereader",
        sum = "h1:ygSAOl7ZXTx4RdPYinUpg6W99U8jWvWi9Ye2JC/oIi4=",
        version = "v0.0.1",
    )
    go_repository(
        name = "com_github_mattn_go_runewidth",
        build_file_proto_mode = "disable_global",
        importpath = "github.com/mattn/go-runewidth",
        sum = "h1:E5ScNMtiwvlvB5paMFdw9p4kSQzbXFikJ5SQO6TULQc=",
        version = "v0.0.16",
    )
    go_repository(
        name = "com_github_microcosm_cc_bluemonday",
        build_file_proto_mode = "disable_global",
        importpath = "github.com/microcosm-cc/bluemonday",
        sum = "h1:MpEUotklkwCSLeH+Qdx1VJgNqLlpY2KXwXFM08ygZfk=",
        version = "v1.0.27",
    )
    go_repository(
        name = "com_github_microsoft_go_winio",
        build_file_proto_mode = "disable_global",
        importpath = "github.com/Microsoft/go-winio",
        sum = "h1:F2VQgta7ecxGYO8k3ZZz3RS8fVIXVxONVUPlNERoyfY=",
        version = "v0.6.2",
    )
    go_repository(
        name = "com_github_mitchellh_copystructure",
        build_file_proto_mode = "disable_global",
        importpath = "github.com/mitchellh/copystructure",
        sum = "h1:vpKXTN4ewci03Vljg/q9QvCGUDttBOGBIa15WveJJGw=",
        version = "v1.2.0",
    )
    go_repository(
        name = "com_github_mitchellh_go_homedir",
        build_file_proto_mode = "disable_global",
        importpath = "github.com/mitchellh/go-homedir",
        sum = "h1:lukF9ziXFxDFPkA1vsr5zpc1XuPDn/wFntq5mG+4E0Y=",
        version = "v1.1.0",
    )
    go_repository(
        name = "com_github_mitchellh_go_testing_interface",
        build_file_proto_mode = "disable_global",
        importpath = "github.com/mitchellh/go-testing-interface",
        sum = "h1:jrgshOhYAUVNMAJiKbEu7EqAwgJJ2JqpQmpLJOu07cU=",
        version = "v1.14.1",
    )
    go_repository(
        name = "com_github_mitchellh_hashstructure_v2",
        build_file_proto_mode = "disable_global",
        importpath = "github.com/mitchellh/hashstructure/v2",
        sum = "h1:vGKWl0YJqUNxE8d+h8f6NJLcCJrgbhC4NcD46KavDd4=",
        version = "v2.0.2",
    )
    go_repository(
        name = "com_github_mitchellh_mapstructure",
        build_file_proto_mode = "disable_global",
        importpath = "github.com/mitchellh/mapstructure",
        sum = "h1:jeMsZIYE/09sWLaz43PL7Gy6RuMjD2eJVyuac5Z2hdY=",
        version = "v1.5.0",
    )
    go_repository(
        name = "com_github_mitchellh_reflectwalk",
        build_file_proto_mode = "disable_global",
        importpath = "github.com/mitchellh/reflectwalk",
        sum = "h1:G2LzWKi524PWgd3mLHV8Y5k7s6XUvT0Gef6zxSIeXaQ=",
        version = "v1.0.2",
    )
    go_repository(
        name = "com_github_modern_go_concurrent",
        build_file_proto_mode = "disable_global",
        importpath = "github.com/modern-go/concurrent",
        sum = "h1:TRLaZ9cD/w8PVh93nsPXa1VrQ6jlwL5oN8l14QlcNfg=",
        version = "v0.0.0-20180306012644-bacd9c7ef1dd",
    )
    go_repository(
        name = "com_github_modern_go_reflect2",
        build_file_proto_mode = "disable_global",
        importpath = "github.com/modern-go/reflect2",
        sum = "h1:xBagoLtFs94CBntxluKeaWgTMpvLxC4ur3nMaC9Gz0M=",
        version = "v1.0.2",
    )
    go_repository(
        name = "com_github_msolo_jsonr",
        build_file_proto_mode = "disable_global",
        importpath = "github.com/msolo/jsonr",
        sum = "h1:A8w3/wSixa6RqN8bLYQ9mayk+d8xxq6jQCmlKs/jCRo=",
        version = "v0.0.0-20231023064044-62fbfc3a0313",
    )
    go_repository(
        name = "com_github_muesli_ansi",
        build_file_proto_mode = "disable_global",
        importpath = "github.com/muesli/ansi",
        sum = "h1:ZK8zHtRHOkbHy6Mmr5D264iyp3TiX5OmNcI5cIARiQI=",
        version = "v0.0.0-20230316100256-276c6243b2f6",
    )
    go_repository(
        name = "com_github_muesli_cancelreader",
        build_file_proto_mode = "disable_global",
        importpath = "github.com/muesli/cancelreader",
        sum = "h1:3I4Kt4BQjOR54NavqnDogx/MIoWBFa0StPA8ELUXHmA=",
        version = "v0.2.2",
    )
    go_repository(
        name = "com_github_muesli_reflow",
        build_file_proto_mode = "disable_global",
        importpath = "github.com/muesli/reflow",
        sum = "h1:IFsN6K9NfGtjeggFP+68I4chLZV2yIKsXJFNZ+eWh6s=",
        version = "v0.3.0",
    )
    go_repository(
        name = "com_github_muesli_termenv",
        build_file_proto_mode = "disable_global",
        importpath = "github.com/muesli/termenv",
        sum = "h1:2MaM6YC3mGu54x+RKAA6JiFFHlHDY1UbkxqppT7wYOg=",
        version = "v0.15.3-0.20240618155329-98d742f6907a",
    )
    go_repository(
        name = "com_github_nats_io_nats_go",
        build_file_proto_mode = "disable_global",
        importpath = "github.com/nats-io/nats.go",
        sum = "h1:fnxnPCNiwIG5w08rlMcEKTUw4AV/nKyGCOJE8TdhSPk=",
        version = "v1.34.0",
    )
    go_repository(
        name = "com_github_nats_io_nkeys",
        build_file_proto_mode = "disable_global",
        importpath = "github.com/nats-io/nkeys",
        sum = "h1:RwNJbbIdYCoClSDNY7QVKZlyb/wfT6ugvFCiKy6vDvI=",
        version = "v0.4.7",
    )
    go_repository(
        name = "com_github_nats_io_nuid",
        build_file_proto_mode = "disable_global",
        importpath = "github.com/nats-io/nuid",
        sum = "h1:5iA8DT8V7q8WK2EScv2padNa/rTESc1KdnPw4TC2paw=",
        version = "v1.0.1",
    )
    go_repository(
        name = "com_github_oklog_run",
        build_file_proto_mode = "disable_global",
        importpath = "github.com/oklog/run",
        sum = "h1:GEenZ1cK0+q0+wsJew9qUg/DyD8k3JzYsZAi5gYi2mA=",
        version = "v1.1.0",
    )
    go_repository(
        name = "com_github_onsi_ginkgo_v2",
        build_file_proto_mode = "disable_global",
        importpath = "github.com/onsi/ginkgo/v2",
        sum = "h1:9Cnnf7UHo57Hy3k6/m5k3dRfGTMXGvxhHFvkDTCTpvA=",
        version = "v2.19.0",
    )
    go_repository(
        name = "com_github_onsi_gomega",
        build_file_proto_mode = "disable_global",
        importpath = "github.com/onsi/gomega",
        sum = "h1:EUMJIKUjM8sKjYbtxQI9A4z2o+rruxnzNvpknOXie6k=",
        version = "v1.34.1",
    )
    go_repository(
        name = "com_github_pelletier_go_toml_v2",
        build_file_proto_mode = "disable_global",
        importpath = "github.com/pelletier/go-toml/v2",
        sum = "h1:aYUidT7k73Pcl9nb2gScu7NSrKCSHIDE89b3+6Wq+LM=",
        version = "v2.2.2",
    )
    go_repository(
        name = "com_github_pjbgf_sha1cd",
        build_file_proto_mode = "disable_global",
        importpath = "github.com/pjbgf/sha1cd",
        sum = "h1:a9wb0bp1oC2TGwStyn0Umc/IGKQnEgF0vVaZ8QF8eo4=",
        version = "v0.3.2",
    )
    go_repository(
        name = "com_github_pkg_browser",
        build_file_proto_mode = "disable_global",
        importpath = "github.com/pkg/browser",
        sum = "h1:+mdjkGKdHQG3305AYmdv1U2eRNDiU2ErMBj1gwrq8eQ=",
        version = "v0.0.0-20240102092130-5ac0b6a4141c",
    )
    go_repository(
        name = "com_github_pkg_errors",
        build_file_proto_mode = "disable_global",
        importpath = "github.com/pkg/errors",
        sum = "h1:FEBLx1zS214owpjy7qsBeixbURkuhQAwrK5UwLGTwt4=",
        version = "v0.9.1",
    )
    go_repository(
        name = "com_github_pkg_sftp",
        build_file_proto_mode = "disable_global",
        importpath = "github.com/pkg/sftp",
        sum = "h1:JFZT4XbOU7l77xGSpOdW+pwIMqP044IyjXX6FGyEKFo=",
        version = "v1.13.6",
    )
    go_repository(
        name = "com_github_pmezard_go_difflib",
        build_file_proto_mode = "disable_global",
        importpath = "github.com/pmezard/go-difflib",
        sum = "h1:Jamvg5psRIccs7FGNTlIRMkT8wgtp5eCXdBlqhYGL6U=",
        version = "v1.0.1-0.20181226105442-5d4384ee4fb2",
    )
    go_repository(
        name = "com_github_protonmail_go_crypto",
        build_file_proto_mode = "disable_global",
        importpath = "github.com/ProtonMail/go-crypto",
        sum = "h1:eoAQfK2dwL+tFSFpr7TbOaPNUbPiJj4fLYwwGE1FQO4=",
        version = "v1.1.5",
    )
    go_repository(
        name = "com_github_psanford_memfs",
        build_file_proto_mode = "disable_global",
        importpath = "github.com/psanford/memfs",
        sum = "h1:xzjEJAHum+mV5Dd5KyohRlCyP03o4yq6vNpEUtAJQzI=",
        version = "v0.0.0-20241019191636-4ef911798f9b",
    )
    go_repository(
        name = "com_github_rivo_uniseg",
        build_file_proto_mode = "disable_global",
        importpath = "github.com/rivo/uniseg",
        sum = "h1:WUdvkW8uEhrYfLC4ZzdpI2ztxP1I582+49Oc5Mq64VQ=",
        version = "v0.4.7",
    )
    go_repository(
        name = "com_github_rogpeppe_go_internal",
        build_file_proto_mode = "disable_global",
        importpath = "github.com/rogpeppe/go-internal",
        sum = "h1:exVL4IDcn6na9z1rAb56Vxr+CgyK3nn3O+epU5NdKM8=",
        version = "v1.12.0",
    )
    go_repository(
        name = "com_github_rs_xid",
        build_file_proto_mode = "disable_global",
        importpath = "github.com/rs/xid",
        sum = "h1:mKX4bl4iPYJtEIxp6CYiUuLQ/8DYMoz0PUdtGgMFRVc=",
        version = "v1.5.0",
    )
    go_repository(
        name = "com_github_rs_zerolog",
        build_file_proto_mode = "disable_global",
        importpath = "github.com/rs/zerolog",
        sum = "h1:1cU2KZkvPxNyfgEmhHAz/1A9Bz+llsdYzklWFzgp0r8=",
        version = "v1.33.0",
    )
    go_repository(
        name = "com_github_russross_blackfriday_v2",
        build_file_proto_mode = "disable_global",
        importpath = "github.com/russross/blackfriday/v2",
        sum = "h1:JIOH55/0cWyOuilr9/qlrm0BSXldqnqwMsf35Ld67mk=",
        version = "v2.1.0",
    )
    go_repository(
        name = "com_github_sagikazarmark_crypt",
        build_file_proto_mode = "disable_global",
        importpath = "github.com/sagikazarmark/crypt",
        sum = "h1:WMyLTjHBo64UvNcWqpzY3pbZTYgnemZU8FBZigKc42E=",
        version = "v0.19.0",
    )
    go_repository(
        name = "com_github_sagikazarmark_locafero",
        build_file_proto_mode = "disable_global",
        importpath = "github.com/sagikazarmark/locafero",
        sum = "h1:HApY1R9zGo4DBgr7dqsTH/JJxLTTsOt7u6keLGt6kNQ=",
        version = "v0.4.0",
    )
    go_repository(
        name = "com_github_sagikazarmark_slog_shim",
        build_file_proto_mode = "disable_global",
        importpath = "github.com/sagikazarmark/slog-shim",
        sum = "h1:diDBnUNK9N/354PgrxMywXnAwEr1QZcOr6gto+ugjYE=",
        version = "v0.1.0",
    )
    go_repository(
        name = "com_github_sahilm_fuzzy",
        build_file_proto_mode = "disable_global",
        importpath = "github.com/sahilm/fuzzy",
        sum = "h1:ceu5RHF8DGgoi+/dR5PsECjCDH1BE3Fnmpo7aVXOdRA=",
        version = "v0.1.1",
    )
    go_repository(
        name = "com_github_sergi_go_diff",
        build_file_proto_mode = "disable_global",
        importpath = "github.com/sergi/go-diff",
        sum = "h1:n661drycOFuPLCN3Uc8sB6B/s6Z4t2xvBgU1htSHuq8=",
        version = "v1.3.2-0.20230802210424-5b0b94c5c0d3",
    )
    go_repository(
        name = "com_github_shurcool_go",
        build_file_proto_mode = "disable_global",
        importpath = "github.com/shurcooL/go",
        sum = "h1:MZM7FHLqUHYI0Y/mQAt3d2aYa0SiNms/hFqC9qJYolM=",
        version = "v0.0.0-20180423040247-9e1955d9fb6e",
    )
    go_repository(
        name = "com_github_shurcool_go_goon",
        build_file_proto_mode = "disable_global",
        importpath = "github.com/shurcooL/go-goon",
        sum = "h1:llrF3Fs4018ePo4+G/HV/uQUqEI1HMDjCeOf2V6puPc=",
        version = "v0.0.0-20170922171312-37c2f522c041",
    )
    go_repository(
        name = "com_github_sirupsen_logrus",
        build_file_proto_mode = "disable_global",
        importpath = "github.com/sirupsen/logrus",
        sum = "h1:dueUQJ1C2q9oE3F7wvmSGAaVtTmUizReu6fjN8uqzbQ=",
        version = "v1.9.3",
    )
    go_repository(
        name = "com_github_skeema_knownhosts",
        build_file_proto_mode = "disable_global",
        importpath = "github.com/skeema/knownhosts",
        sum = "h1:X2osQ+RAjK76shCbvhHHHVl3ZlgDm8apHEHFqRjnBY8=",
        version = "v1.3.1",
    )
    go_repository(
        name = "com_github_smacker_go_tree_sitter",
        build_file_proto_mode = "disable_global",
        importpath = "github.com/smacker/go-tree-sitter",
        patches = ["//third_party:github.com/smacker/go-tree-sitter/cc_library.patch"],  # keep
        sum = "h1:6C8qej6f1bStuePVkLSFxoU22XBS165D3klxlzRg8F4=",
        version = "v0.0.0-20240827094217-dd81d9e9be82",
    )
    go_repository(
        name = "com_github_sourcegraph_conc",
        build_file_proto_mode = "disable_global",
        importpath = "github.com/sourcegraph/conc",
        sum = "h1:OQTbbt6P72L20UqAkXXuLOj79LfEanQ+YQFNpLA9ySo=",
        version = "v0.3.0",
    )
    go_repository(
        name = "com_github_sourcegraph_go_diff",
        build_file_proto_mode = "disable_global",
        importpath = "github.com/sourcegraph/go-diff",
        sum = "h1:9uLlrd5T46OXs5qpp8L/MTltk0zikUGi0sNNyCpA8G0=",
        version = "v0.7.0",
    )
    go_repository(
        name = "com_github_spf13_afero",
        build_file_proto_mode = "disable_global",
        importpath = "github.com/spf13/afero",
        sum = "h1:WJQKhtpdm3v2IzqG8VMqrr6Rf3UYpEF239Jy9wNepM8=",
        version = "v1.11.0",
    )
    go_repository(
        name = "com_github_spf13_cast",
        build_file_proto_mode = "disable_global",
        importpath = "github.com/spf13/cast",
        sum = "h1:cuNEagBQEHWN1FnbGEjCXL2szYEXqfJPbP2HNUaca9Y=",
        version = "v1.7.1",
    )
    go_repository(
        name = "com_github_spf13_cobra",
        build_file_proto_mode = "disable_global",
        importpath = "github.com/spf13/cobra",
        sum = "h1:e5/vxKd/rZsfSJMUX1agtjeTDf+qv1/JdBF8gg5k9ZM=",
        version = "v1.8.1",
    )
    go_repository(
        name = "com_github_spf13_pflag",
        build_file_proto_mode = "disable_global",
        importpath = "github.com/spf13/pflag",
        sum = "h1:iy+VFUOCP1a+8yFto/drg2CJ5u0yRoB7fZw3DKv/JXA=",
        version = "v1.0.5",
    )
    go_repository(
        name = "com_github_spf13_viper",
        build_file_proto_mode = "disable_global",
        importpath = "github.com/spf13/viper",
        sum = "h1:RWq5SEjt8o25SROyN3z2OrDB9l7RPd3lwTWU8EcEdcI=",
        version = "v1.19.0",
    )
    go_repository(
        name = "com_github_stretchr_objx",
        build_file_proto_mode = "disable_global",
        importpath = "github.com/stretchr/objx",
        sum = "h1:xuMeJ0Sdp5ZMRXx/aWO6RZxdr3beISkG5/G/aIRr3pY=",
        version = "v0.5.2",
    )
    go_repository(
        name = "com_github_stretchr_testify",
        build_file_proto_mode = "disable_global",
        importpath = "github.com/stretchr/testify",
        sum = "h1:Xv5erBjTwe/5IxqUQTdXv5kgmIvbHo3QQyRwhJsOfJA=",
        version = "v1.10.0",
    )
    go_repository(
        name = "com_github_subosito_gotenv",
        build_file_proto_mode = "disable_global",
        importpath = "github.com/subosito/gotenv",
        sum = "h1:9NlTDc1FTs4qu0DDq7AEtTPNw6SVm7uBMsUCUjABIf8=",
        version = "v1.6.0",
    )
    go_repository(
        name = "com_github_tejzpr_ordered_concurrently_v3",
        build_file_proto_mode = "disable_global",
        importpath = "github.com/tejzpr/ordered-concurrently/v3",
        sum = "h1:TLHtzlQEDshbmGveS8S+hxLw4s5u67aoJw5LLf+X2xY=",
        version = "v3.0.1",
    )
    go_repository(
        name = "com_github_twmb_murmur3",
        build_file_proto_mode = "disable_global",
        importpath = "github.com/twmb/murmur3",
        sum = "h1:8Yt9taO/WN3l08xErzjeschgZU2QSrwm1kclYq+0aRg=",
        version = "v1.1.8",
    )
    go_repository(
        name = "com_github_urfave_cli_v2",
        build_file_proto_mode = "disable_global",
        importpath = "github.com/urfave/cli/v2",
        sum = "h1:WoHEJLdsXr6dDWoJgMq/CboDmyY/8HMMH1fTECbih+w=",
        version = "v2.27.5",
    )
    go_repository(
        name = "com_github_xanzy_ssh_agent",
        build_file_proto_mode = "disable_global",
        importpath = "github.com/xanzy/ssh-agent",
        sum = "h1:+/15pJfg/RsTxqYcX6fHqOXZwwMP+2VyYWJeWM2qQFM=",
        version = "v0.3.3",
    )
    go_repository(
        name = "com_github_xrash_smetrics",
        build_file_proto_mode = "disable_global",
        importpath = "github.com/xrash/smetrics",
        sum = "h1:gEOO8jv9F4OT7lGCjxCBTO/36wtF6j2nSip77qHd4x4=",
        version = "v0.0.0-20240521201337-686a1a2994c1",
    )
    go_repository(
        name = "com_github_yuin_goldmark",
        build_file_proto_mode = "disable_global",
        importpath = "github.com/yuin/goldmark",
        sum = "h1:iERMLn0/QJeHFhxSt3p6PeN9mGnvIKSpG9YYorDMnic=",
        version = "v1.7.8",
    )
    go_repository(
        name = "com_github_yuin_goldmark_emoji",
        build_file_proto_mode = "disable_global",
        importpath = "github.com/yuin/goldmark-emoji",
        sum = "h1:vCwMkPZSNefSUnOW2ZKRUjBSD5Ok3W78IXhGxxAEF90=",
        version = "v1.0.4",
    )
    go_repository(
        name = "com_google_cloud_go",
        build_file_proto_mode = "disable_global",
        importpath = "cloud.google.com/go",
        sum = "h1:OIPFAdfrFDFO2ve2U7r/H5SwSbBzEdrBdE7xkgwc+kY=",
        version = "v0.114.0",
    )
    go_repository(
        name = "com_google_cloud_go_accessapproval",
        build_file_proto_mode = "disable_global",
        importpath = "cloud.google.com/go/accessapproval",
        sum = "h1:vO95gvBi7qUgfA9SflexQs9hB4U4tnri/GwADIrLQy8=",
        version = "v1.7.7",
    )
    go_repository(
        name = "com_google_cloud_go_accesscontextmanager",
        build_file_proto_mode = "disable_global",
        importpath = "cloud.google.com/go/accesscontextmanager",
        sum = "h1:GgdNoDwZR5RIO3j8XwXqa6Gc6q5mP3KYMdFC7FEVyG4=",
        version = "v1.8.7",
    )
    go_repository(
        name = "com_google_cloud_go_aiplatform",
        build_file_proto_mode = "disable_global",
        importpath = "cloud.google.com/go/aiplatform",
        sum = "h1:YWeqD4BjYwrmY4fa+isGcw0P81lJ3dKVxbWxdBchoiU=",
        version = "v1.67.0",
    )
    go_repository(
        name = "com_google_cloud_go_analytics",
        build_file_proto_mode = "disable_global",
        importpath = "cloud.google.com/go/analytics",
        sum = "h1:O0fj88npvQFxg8LfXo7fArcSrC/wtAstGuWQ7dCHWjg=",
        version = "v0.23.2",
    )
    go_repository(
        name = "com_google_cloud_go_apigateway",
        build_file_proto_mode = "disable_global",
        importpath = "cloud.google.com/go/apigateway",
        sum = "h1:DO5Vn3zmY1aDyfoqni8e8+x+lwrfLCoAAbEui9NB0y8=",
        version = "v1.6.7",
    )
    go_repository(
        name = "com_google_cloud_go_apigeeconnect",
        build_file_proto_mode = "disable_global",
        importpath = "cloud.google.com/go/apigeeconnect",
        sum = "h1:z08Xuv7ZtaB2d4jsJi9/WhbnnI5s19wlLDZpssn3Fus=",
        version = "v1.6.7",
    )
    go_repository(
        name = "com_google_cloud_go_apigeeregistry",
        build_file_proto_mode = "disable_global",
        importpath = "cloud.google.com/go/apigeeregistry",
        sum = "h1:o1C/+IvzwYeV1doum61XmJQ/Bwpk/4+2DT1JyVu2x64=",
        version = "v0.8.5",
    )
    go_repository(
        name = "com_google_cloud_go_appengine",
        build_file_proto_mode = "disable_global",
        importpath = "cloud.google.com/go/appengine",
        sum = "h1:qYrjEHEFY7+CL4QlHIHuwTgrTnZbSKzdPFqgjZDsQNo=",
        version = "v1.8.7",
    )
    go_repository(
        name = "com_google_cloud_go_area120",
        build_file_proto_mode = "disable_global",
        importpath = "cloud.google.com/go/area120",
        sum = "h1:sUrR96yokdL6tTTXK0X13V1TLMta8/1u328bRG5lWZc=",
        version = "v0.8.7",
    )
    go_repository(
        name = "com_google_cloud_go_artifactregistry",
        build_file_proto_mode = "disable_global",
        importpath = "cloud.google.com/go/artifactregistry",
        sum = "h1:SSvoD0ofOydm5gA1++15pW9VPgQbk0OmNlcb7JczoO4=",
        version = "v1.14.9",
    )
    go_repository(
        name = "com_google_cloud_go_asset",
        build_file_proto_mode = "disable_global",
        importpath = "cloud.google.com/go/asset",
        sum = "h1:mCqyoaDjDzaW1RqmmQtCJuawb9nca5bEu7HvVcpZDwg=",
        version = "v1.19.1",
    )
    go_repository(
        name = "com_google_cloud_go_assuredworkloads",
        build_file_proto_mode = "disable_global",
        importpath = "cloud.google.com/go/assuredworkloads",
        sum = "h1:xieyFA+JKyTDkO/Z9UyVEpkHW8pDYykU51O4G0pvXEg=",
        version = "v1.11.7",
    )
    go_repository(
        name = "com_google_cloud_go_automl",
        build_file_proto_mode = "disable_global",
        importpath = "cloud.google.com/go/automl",
        sum = "h1:w9AyogtMLXbcy5kzXPvk/Q3MGQkgJH7ZDB8fAUUxTt8=",
        version = "v1.13.7",
    )
    go_repository(
        name = "com_google_cloud_go_baremetalsolution",
        build_file_proto_mode = "disable_global",
        importpath = "cloud.google.com/go/baremetalsolution",
        sum = "h1:W4oSMS6vRCo9DLr1RPyDP8oeLverbvhJRzaZSsipft8=",
        version = "v1.2.6",
    )
    go_repository(
        name = "com_google_cloud_go_batch",
        build_file_proto_mode = "disable_global",
        importpath = "cloud.google.com/go/batch",
        sum = "h1:yjAV0wkKYqO5Lh20dGdq5HDzQDG2qWWfbirLohNbpn0=",
        version = "v1.8.6",
    )
    go_repository(
        name = "com_google_cloud_go_beyondcorp",
        build_file_proto_mode = "disable_global",
        importpath = "cloud.google.com/go/beyondcorp",
        sum = "h1:KBcujO3QRvBIwzZLtvQEPB9SXdovHnMBx0V/uhucH9o=",
        version = "v1.0.6",
    )
    go_repository(
        name = "com_google_cloud_go_bigquery",
        build_file_proto_mode = "disable_global",
        importpath = "cloud.google.com/go/bigquery",
        sum = "h1:w2Goy9n6gh91LVi6B2Sc+HpBl8WbWhIyzdvVvrAuEIw=",
        version = "v1.61.0",
    )
    go_repository(
        name = "com_google_cloud_go_billing",
        build_file_proto_mode = "disable_global",
        importpath = "cloud.google.com/go/billing",
        sum = "h1:GbOg1uGvoV8FXxMStFoNcq5z9AEUwCpKt/6GNcuDSZM=",
        version = "v1.18.5",
    )
    go_repository(
        name = "com_google_cloud_go_binaryauthorization",
        build_file_proto_mode = "disable_global",
        importpath = "cloud.google.com/go/binaryauthorization",
        sum = "h1:RHnEM4HXbWShlGhPA0Jzj2YYETCHxmisNMU0OE2fXQM=",
        version = "v1.8.3",
    )
    go_repository(
        name = "com_google_cloud_go_certificatemanager",
        build_file_proto_mode = "disable_global",
        importpath = "cloud.google.com/go/certificatemanager",
        sum = "h1:XURrQhj5COWAEvICivbGID/Hu67AvMYHAhMRIyc3Ux8=",
        version = "v1.8.1",
    )
    go_repository(
        name = "com_google_cloud_go_channel",
        build_file_proto_mode = "disable_global",
        importpath = "cloud.google.com/go/channel",
        sum = "h1:PrplNaAS6Dn187e+OcGzyEKETX8iL3tCaDqcPPW7Zoo=",
        version = "v1.17.7",
    )
    go_repository(
        name = "com_google_cloud_go_cloudbuild",
        build_file_proto_mode = "disable_global",
        importpath = "cloud.google.com/go/cloudbuild",
        sum = "h1:zkCG1dBezxRM3dtgQ9h1Y+IJ7V+lARWgp0l9k/SZsfU=",
        version = "v1.16.1",
    )
    go_repository(
        name = "com_google_cloud_go_clouddms",
        build_file_proto_mode = "disable_global",
        importpath = "cloud.google.com/go/clouddms",
        sum = "h1:Q47KKoA0zsNcC9U5aCmop5TPPItVq4cx7Wwqgra+5PU=",
        version = "v1.7.6",
    )
    go_repository(
        name = "com_google_cloud_go_cloudtasks",
        build_file_proto_mode = "disable_global",
        importpath = "cloud.google.com/go/cloudtasks",
        sum = "h1:Y0HUuiCAVk9BojLItOycBl91tY25NXH8oFsyi1IC/U4=",
        version = "v1.12.8",
    )
    go_repository(
        name = "com_google_cloud_go_compute",
        build_file_proto_mode = "disable_global",
        importpath = "cloud.google.com/go/compute",
        sum = "h1:EGawh2RUnfHT5g8f/FX3Ds6KZuIBC77hZoDrBvEZw94=",
        version = "v1.27.0",
    )
    go_repository(
        name = "com_google_cloud_go_compute_metadata",
        build_file_proto_mode = "disable_global",
        importpath = "cloud.google.com/go/compute/metadata",
        sum = "h1:mg4jlk7mCAj6xXp9UJ4fjI9VUI5rubuGBW5aJ7UnBMY=",
        version = "v0.2.3",
    )
    go_repository(
        name = "com_google_cloud_go_contactcenterinsights",
        build_file_proto_mode = "disable_global",
        importpath = "cloud.google.com/go/contactcenterinsights",
        sum = "h1:46ertIh+cGkTg/lN7fN+TOx09SoM65dpdUp96vXBcMY=",
        version = "v1.13.2",
    )
    go_repository(
        name = "com_google_cloud_go_container",
        build_file_proto_mode = "disable_global",
        importpath = "cloud.google.com/go/container",
        sum = "h1:Vbu/3PZNrgV1Z5DGcRubQdUccX/uMUDNc+NgHNIfbEk=",
        version = "v1.35.1",
    )
    go_repository(
        name = "com_google_cloud_go_containeranalysis",
        build_file_proto_mode = "disable_global",
        importpath = "cloud.google.com/go/containeranalysis",
        sum = "h1:mSrneOVadcpnDZHJebg+ts/10azGTUKOCSQET7KdT7g=",
        version = "v0.11.6",
    )
    go_repository(
        name = "com_google_cloud_go_datacatalog",
        build_file_proto_mode = "disable_global",
        importpath = "cloud.google.com/go/datacatalog",
        sum = "h1:czcba5mxwRM5V//jSadyig0y+8aOHmN7gUl9GbHu59E=",
        version = "v1.20.1",
    )
    go_repository(
        name = "com_google_cloud_go_dataflow",
        build_file_proto_mode = "disable_global",
        importpath = "cloud.google.com/go/dataflow",
        sum = "h1:wKEakCbRevlwsWqTn34pWJUFmdbx0HKwpRH6HhU7NIs=",
        version = "v0.9.7",
    )
    go_repository(
        name = "com_google_cloud_go_dataform",
        build_file_proto_mode = "disable_global",
        importpath = "cloud.google.com/go/dataform",
        sum = "h1:MiK1Us7YP9+sdNViUE4X2B2vLScrKcjOPw5b6uamZvE=",
        version = "v0.9.4",
    )
    go_repository(
        name = "com_google_cloud_go_datafusion",
        build_file_proto_mode = "disable_global",
        importpath = "cloud.google.com/go/datafusion",
        sum = "h1:ViFnMnUK7LNcWvisZgihxXit76JxSHFeijYI5U/gjOE=",
        version = "v1.7.7",
    )
    go_repository(
        name = "com_google_cloud_go_datalabeling",
        build_file_proto_mode = "disable_global",
        importpath = "cloud.google.com/go/datalabeling",
        sum = "h1:M6irSHns6VxMro+IbvDxDJLD6tkfjlW+mo2MPaM23KA=",
        version = "v0.8.7",
    )
    go_repository(
        name = "com_google_cloud_go_dataplex",
        build_file_proto_mode = "disable_global",
        importpath = "cloud.google.com/go/dataplex",
        sum = "h1:e8SV0yKuSjgHEZaQcZwjKXe0ta1jZrvLxX/2i/IAG+8=",
        version = "v1.16.0",
    )
    go_repository(
        name = "com_google_cloud_go_dataproc_v2",
        build_file_proto_mode = "disable_global",
        importpath = "cloud.google.com/go/dataproc/v2",
        sum = "h1:RNMG5ffWKdbWOkwvjC4GqxLaxEaWFpm2hQCF2WFW/vo=",
        version = "v2.4.2",
    )
    go_repository(
        name = "com_google_cloud_go_dataqna",
        build_file_proto_mode = "disable_global",
        importpath = "cloud.google.com/go/dataqna",
        sum = "h1:qM60MGNTGsSJuzAziVJjtRA7pGby2dA8OuqdVRe/lYo=",
        version = "v0.8.7",
    )
    go_repository(
        name = "com_google_cloud_go_datastore",
        build_file_proto_mode = "disable_global",
        importpath = "cloud.google.com/go/datastore",
        sum = "h1:UEmzuUdyDE58HV2jcb0BoqwCAwsJS2mtHapCsMmhVh0=",
        version = "v1.17.0",
    )
    go_repository(
        name = "com_google_cloud_go_datastream",
        build_file_proto_mode = "disable_global",
        importpath = "cloud.google.com/go/datastream",
        sum = "h1:FfNUy9j3aRQ99L4a5Rdm82RMuiw0BIe3lpPn2ykom8k=",
        version = "v1.10.6",
    )
    go_repository(
        name = "com_google_cloud_go_deploy",
        build_file_proto_mode = "disable_global",
        importpath = "cloud.google.com/go/deploy",
        sum = "h1:fzbObuGgoViO0ArFuOQIJ2yr5bH5YzbORVvMDBrDC5I=",
        version = "v1.19.0",
    )
    go_repository(
        name = "com_google_cloud_go_dialogflow",
        build_file_proto_mode = "disable_global",
        importpath = "cloud.google.com/go/dialogflow",
        sum = "h1:C9wQ0odRYQsar0XqwCQb0c13BkRBsoSjOaejOg5ntgQ=",
        version = "v1.53.0",
    )
    go_repository(
        name = "com_google_cloud_go_dlp",
        build_file_proto_mode = "disable_global",
        importpath = "cloud.google.com/go/dlp",
        sum = "h1:T7p+nTgLyp++5WcYAYD9+hVA5TsYWD4zasD5LsoyhvU=",
        version = "v1.13.0",
    )
    go_repository(
        name = "com_google_cloud_go_documentai",
        build_file_proto_mode = "disable_global",
        importpath = "cloud.google.com/go/documentai",
        sum = "h1:UrEp5LASw0zIxBJaVsXSCHYpsMB9YcVZ66CPOmJQWyo=",
        version = "v1.28.1",
    )
    go_repository(
        name = "com_google_cloud_go_domains",
        build_file_proto_mode = "disable_global",
        importpath = "cloud.google.com/go/domains",
        sum = "h1:IixFIMRzUJWZUAOe8s/K2X4Bvtp0A3xjHLljfNC4aSo=",
        version = "v0.9.7",
    )
    go_repository(
        name = "com_google_cloud_go_edgecontainer",
        build_file_proto_mode = "disable_global",
        importpath = "cloud.google.com/go/edgecontainer",
        sum = "h1:xa6MIQhGylE24QdWaxhfIfAJE3Pupcr+i77WEx3NJrg=",
        version = "v1.2.1",
    )
    go_repository(
        name = "com_google_cloud_go_errorreporting",
        build_file_proto_mode = "disable_global",
        importpath = "cloud.google.com/go/errorreporting",
        sum = "h1:kj1XEWMu8P0qlLhm3FwcaFsUvXChV/OraZwA70trRR0=",
        version = "v0.3.0",
    )
    go_repository(
        name = "com_google_cloud_go_essentialcontacts",
        build_file_proto_mode = "disable_global",
        importpath = "cloud.google.com/go/essentialcontacts",
        sum = "h1:p5Y7ZNVPiV9pEAHzvWiPcSiQRMQqcuHxOP0ZOP0vVww=",
        version = "v1.6.8",
    )
    go_repository(
        name = "com_google_cloud_go_eventarc",
        build_file_proto_mode = "disable_global",
        importpath = "cloud.google.com/go/eventarc",
        sum = "h1:we+qx5uCZ88aQzQS3MJXRvAh/ik+EmqVyjcW1oYFW44=",
        version = "v1.13.6",
    )
    go_repository(
        name = "com_google_cloud_go_filestore",
        build_file_proto_mode = "disable_global",
        importpath = "cloud.google.com/go/filestore",
        sum = "h1:CpRnsUpMU5gxUKyfh7TD0SM+E+7E4ORaDea2JctKfpY=",
        version = "v1.8.3",
    )
    go_repository(
        name = "com_google_cloud_go_firestore",
        build_file_proto_mode = "disable_global",
        importpath = "cloud.google.com/go/firestore",
        sum = "h1:/k8ppuWOtNuDHt2tsRV42yI21uaGnKDEQnRFeBpbFF8=",
        version = "v1.15.0",
    )
    go_repository(
        name = "com_google_cloud_go_functions",
        build_file_proto_mode = "disable_global",
        importpath = "cloud.google.com/go/functions",
        sum = "h1:83bd2lCgtu2nLbX2jrqsrQhIs7VuVA1N6Op5syeRVIg=",
        version = "v1.16.2",
    )
    go_repository(
        name = "com_google_cloud_go_gkebackup",
        build_file_proto_mode = "disable_global",
        importpath = "cloud.google.com/go/gkebackup",
        sum = "h1:wysUXEkggPwENZY3BXroOyWoyVfPypzaqNHgOZD9Kck=",
        version = "v1.5.0",
    )
    go_repository(
        name = "com_google_cloud_go_gkeconnect",
        build_file_proto_mode = "disable_global",
        importpath = "cloud.google.com/go/gkeconnect",
        sum = "h1:BfXsTXYs5xlicAlgbtlo8Cw+YdzU3PrlBg7dATJUwrk=",
        version = "v0.8.7",
    )
    go_repository(
        name = "com_google_cloud_go_gkehub",
        build_file_proto_mode = "disable_global",
        importpath = "cloud.google.com/go/gkehub",
        sum = "h1:bHwcvgh8AmcYm6p6/ZrWW3a7J7sKBDtqtsyVXKssnPs=",
        version = "v0.14.7",
    )
    go_repository(
        name = "com_google_cloud_go_gkemulticloud",
        build_file_proto_mode = "disable_global",
        importpath = "cloud.google.com/go/gkemulticloud",
        sum = "h1:zaWBakKPT6mPHVn5iefuRqttjpbNsb8LlMw9KgfyfyU=",
        version = "v1.2.0",
    )
    go_repository(
        name = "com_google_cloud_go_gsuiteaddons",
        build_file_proto_mode = "disable_global",
        importpath = "cloud.google.com/go/gsuiteaddons",
        sum = "h1:06Jg3JeLslEfBYX1sDqOPLnF7a3wmhNcDUXF/fVOb50=",
        version = "v1.6.7",
    )
    go_repository(
        name = "com_google_cloud_go_iam",
        build_file_proto_mode = "disable_global",
        importpath = "cloud.google.com/go/iam",
        sum = "h1:r7umDwhj+BQyz0ScZMp4QrGXjSTI3ZINnpgU2nlB/K0=",
        version = "v1.1.8",
    )
    go_repository(
        name = "com_google_cloud_go_iap",
        build_file_proto_mode = "disable_global",
        importpath = "cloud.google.com/go/iap",
        sum = "h1:rcuRS9XfOgr1v6TAoihVeSXntOnpVhFlVHtPfgOkLAo=",
        version = "v1.9.6",
    )
    go_repository(
        name = "com_google_cloud_go_ids",
        build_file_proto_mode = "disable_global",
        importpath = "cloud.google.com/go/ids",
        sum = "h1:wtd+r415yrfZ8LsB6yH6WrOZ26tYt7w6wy3i5a4HQZ8=",
        version = "v1.4.7",
    )
    go_repository(
        name = "com_google_cloud_go_iot",
        build_file_proto_mode = "disable_global",
        importpath = "cloud.google.com/go/iot",
        sum = "h1:M9SKIj9eoxoXCzytkLZVAuf5wmoui1OeDqEjC97wRbY=",
        version = "v1.7.7",
    )
    go_repository(
        name = "com_google_cloud_go_kms",
        build_file_proto_mode = "disable_global",
        importpath = "cloud.google.com/go/kms",
        sum = "h1:5k0wXqkxL+YcXd4viQzTqCgzzVKKxzgrK+rCZJytEQs=",
        version = "v1.17.1",
    )
    go_repository(
        name = "com_google_cloud_go_language",
        build_file_proto_mode = "disable_global",
        importpath = "cloud.google.com/go/language",
        sum = "h1:kOYJEcuZgyUX/i/4DFrfXPcrddm1XCQD2lDI5hIFmZQ=",
        version = "v1.12.5",
    )
    go_repository(
        name = "com_google_cloud_go_lifesciences",
        build_file_proto_mode = "disable_global",
        importpath = "cloud.google.com/go/lifesciences",
        sum = "h1:qqEmApr5YFOQjkrU8Jy6o6QpkESqfGbfrE6bnUZZbV8=",
        version = "v0.9.7",
    )
    go_repository(
        name = "com_google_cloud_go_logging",
        build_file_proto_mode = "disable_global",
        importpath = "cloud.google.com/go/logging",
        sum = "h1:f+ZXMqyrSJ5vZ5pE/zr0xC8y/M9BLNzQeLBwfeZ+wY4=",
        version = "v1.10.0",
    )
    go_repository(
        name = "com_google_cloud_go_longrunning",
        build_file_proto_mode = "disable_global",
        importpath = "cloud.google.com/go/longrunning",
        sum = "h1:WLbHekDbjK1fVFD3ibpFFVoyizlLRl73I7YKuAKilhU=",
        version = "v0.5.7",
    )
    go_repository(
        name = "com_google_cloud_go_managedidentities",
        build_file_proto_mode = "disable_global",
        importpath = "cloud.google.com/go/managedidentities",
        sum = "h1:uWA9WQyfA0JdkeAFymWUsa3qE9tC33LUElla790Ou1A=",
        version = "v1.6.7",
    )
    go_repository(
        name = "com_google_cloud_go_maps",
        build_file_proto_mode = "disable_global",
        importpath = "cloud.google.com/go/maps",
        sum = "h1:0VHFOkgid3256RAwa+RmPRcnAOcqrcXFM4CiFprrXHI=",
        version = "v1.10.0",
    )
    go_repository(
        name = "com_google_cloud_go_mediatranslation",
        build_file_proto_mode = "disable_global",
        importpath = "cloud.google.com/go/mediatranslation",
        sum = "h1:izgww3TlyvWyDWdFKnrASpbh12IkAuw8o2ION8sAjX0=",
        version = "v0.8.7",
    )
    go_repository(
        name = "com_google_cloud_go_memcache",
        build_file_proto_mode = "disable_global",
        importpath = "cloud.google.com/go/memcache",
        sum = "h1:hE7f3ze3+eWh/EbYXEz7oXkm0LXcr7UCoLklwi7gsLU=",
        version = "v1.10.7",
    )
    go_repository(
        name = "com_google_cloud_go_metastore",
        build_file_proto_mode = "disable_global",
        importpath = "cloud.google.com/go/metastore",
        sum = "h1:otHcJkci5f/sNRedrSM7eM81QRnu0yZ3HvkvWGphABA=",
        version = "v1.13.6",
    )
    go_repository(
        name = "com_google_cloud_go_monitoring",
        build_file_proto_mode = "disable_global",
        importpath = "cloud.google.com/go/monitoring",
        sum = "h1:NCXf8hfQi+Kmr56QJezXRZ6GPb80ZI7El1XztyUuLQI=",
        version = "v1.19.0",
    )
    go_repository(
        name = "com_google_cloud_go_networkconnectivity",
        build_file_proto_mode = "disable_global",
        importpath = "cloud.google.com/go/networkconnectivity",
        sum = "h1:jYpQ86mZ7OYZc7WadvCIlIaPXmXhr5nD7wgE/ekMVpM=",
        version = "v1.14.6",
    )
    go_repository(
        name = "com_google_cloud_go_networkmanagement",
        build_file_proto_mode = "disable_global",
        importpath = "cloud.google.com/go/networkmanagement",
        sum = "h1:Ex1/aYkA0areleSmOGXHvEFBGohteIYJr2SGPrjOUe0=",
        version = "v1.13.2",
    )
    go_repository(
        name = "com_google_cloud_go_networksecurity",
        build_file_proto_mode = "disable_global",
        importpath = "cloud.google.com/go/networksecurity",
        sum = "h1:aepEkfiwOvUL9eu3ginVZhTaXDRHncQKi9lTT1BycH0=",
        version = "v0.9.7",
    )
    go_repository(
        name = "com_google_cloud_go_notebooks",
        build_file_proto_mode = "disable_global",
        importpath = "cloud.google.com/go/notebooks",
        sum = "h1:sFU1ETg1HfIN/Tev8gD0dleAITLv7cHp0JClwFmJ6bo=",
        version = "v1.11.5",
    )
    go_repository(
        name = "com_google_cloud_go_optimization",
        build_file_proto_mode = "disable_global",
        importpath = "cloud.google.com/go/optimization",
        sum = "h1:FPfowA/LEckKTQT0A4NJMI2bSou999c2ZyFX1zGiYxY=",
        version = "v1.6.5",
    )
    go_repository(
        name = "com_google_cloud_go_orchestration",
        build_file_proto_mode = "disable_global",
        importpath = "cloud.google.com/go/orchestration",
        sum = "h1:C2WL4ZnclXsh4XickGhKYKlPjqVZj35y1sbRjdsZ3g4=",
        version = "v1.9.2",
    )
    go_repository(
        name = "com_google_cloud_go_orgpolicy",
        build_file_proto_mode = "disable_global",
        importpath = "cloud.google.com/go/orgpolicy",
        sum = "h1:fGftW2bPi8vTjQm57xlwtLBZQcrgC+c3HMFBzJ+KWPc=",
        version = "v1.12.3",
    )
    go_repository(
        name = "com_google_cloud_go_osconfig",
        build_file_proto_mode = "disable_global",
        importpath = "cloud.google.com/go/osconfig",
        sum = "h1:HXsXGFaFaLTklwKgSob/GSE+c3verYDQDgreFaosxyc=",
        version = "v1.12.7",
    )
    go_repository(
        name = "com_google_cloud_go_oslogin",
        build_file_proto_mode = "disable_global",
        importpath = "cloud.google.com/go/oslogin",
        sum = "h1:7AgOWH1oMPrB1AVU0/f47ADdOt+XfdBY7QRb8tcMUp8=",
        version = "v1.13.3",
    )
    go_repository(
        name = "com_google_cloud_go_phishingprotection",
        build_file_proto_mode = "disable_global",
        importpath = "cloud.google.com/go/phishingprotection",
        sum = "h1:CbCjfR/pgDHyRMu94o9nuGwaONEcarWnUfSGGw+I2ZI=",
        version = "v0.8.7",
    )
    go_repository(
        name = "com_google_cloud_go_policytroubleshooter",
        build_file_proto_mode = "disable_global",
        importpath = "cloud.google.com/go/policytroubleshooter",
        sum = "h1:LGt85MZUKlq9oqsbBL9+M6jAyeuR1TtCx6k5HfAQxTY=",
        version = "v1.10.5",
    )
    go_repository(
        name = "com_google_cloud_go_privatecatalog",
        build_file_proto_mode = "disable_global",
        importpath = "cloud.google.com/go/privatecatalog",
        sum = "h1:wGZKKJhYyuf4gcAEywQqQ6F19yxhBJGnzgyxOTbJjBw=",
        version = "v0.9.7",
    )
    go_repository(
        name = "com_google_cloud_go_pubsub",
        build_file_proto_mode = "disable_global",
        importpath = "cloud.google.com/go/pubsub",
        sum = "h1:J1OT7h51ifATIedjqk/uBNPh+1hkvUaH4VKbz4UuAsc=",
        version = "v1.38.0",
    )
    go_repository(
        name = "com_google_cloud_go_pubsublite",
        build_file_proto_mode = "disable_global",
        importpath = "cloud.google.com/go/pubsublite",
        sum = "h1:pX+idpWMIH30/K7c0epN6V703xpIcMXWRjKJsz0tYGY=",
        version = "v1.8.1",
    )
    go_repository(
        name = "com_google_cloud_go_recaptchaenterprise_v2",
        build_file_proto_mode = "disable_global",
        importpath = "cloud.google.com/go/recaptchaenterprise/v2",
        sum = "h1:+QG02kE63W13vXI+rwAxFF3EhGX6K7gXwFz9OKwKcHw=",
        version = "v2.13.0",
    )
    go_repository(
        name = "com_google_cloud_go_recommendationengine",
        build_file_proto_mode = "disable_global",
        importpath = "cloud.google.com/go/recommendationengine",
        sum = "h1:N6n/TEr0FQzeP4ZtvF5daMszOhdZI94uMiPiAi9kFMo=",
        version = "v0.8.7",
    )
    go_repository(
        name = "com_google_cloud_go_recommender",
        build_file_proto_mode = "disable_global",
        importpath = "cloud.google.com/go/recommender",
        sum = "h1:v9x75vXP5wMXw3QG3xmgjVHLlqYufuLn/ht3oNWCA3w=",
        version = "v1.12.3",
    )
    go_repository(
        name = "com_google_cloud_go_redis",
        build_file_proto_mode = "disable_global",
        importpath = "cloud.google.com/go/redis",
        sum = "h1:YB4i013/3jdHgQf+MVUOX4GLNcuHPSWI94QMVc1pSIU=",
        version = "v1.15.0",
    )
    go_repository(
        name = "com_google_cloud_go_resourcemanager",
        build_file_proto_mode = "disable_global",
        importpath = "cloud.google.com/go/resourcemanager",
        sum = "h1:SdvD0PaPX60+yeKoSe16mawFpM0EPuiPPihTIVlhRsY=",
        version = "v1.9.7",
    )
    go_repository(
        name = "com_google_cloud_go_resourcesettings",
        build_file_proto_mode = "disable_global",
        importpath = "cloud.google.com/go/resourcesettings",
        sum = "h1:88SlpWtogkwjMuYTEl//qm36azX1OpawThAyvXT/hHw=",
        version = "v1.6.7",
    )
    go_repository(
        name = "com_google_cloud_go_retail",
        build_file_proto_mode = "disable_global",
        importpath = "cloud.google.com/go/retail",
        sum = "h1:msP5a8BOxVym2DvoubeWAxAeV6VhYkKnYHc2XOkd/+U=",
        version = "v1.16.2",
    )
    go_repository(
        name = "com_google_cloud_go_run",
        build_file_proto_mode = "disable_global",
        importpath = "cloud.google.com/go/run",
        sum = "h1:E4Z5e681Qh7UJrJRMCgYhp+3tkcoXiaKGh3UZmUPaAQ=",
        version = "v1.3.7",
    )
    go_repository(
        name = "com_google_cloud_go_scheduler",
        build_file_proto_mode = "disable_global",
        importpath = "cloud.google.com/go/scheduler",
        sum = "h1:Jn/unfNUgRiNJRc1nrApzimKiVj91UYlLT8mMfpUu48=",
        version = "v1.10.8",
    )
    go_repository(
        name = "com_google_cloud_go_secretmanager",
        build_file_proto_mode = "disable_global",
        importpath = "cloud.google.com/go/secretmanager",
        sum = "h1:TTGo2Vz7ZxYn2QbmuFP7Zo4lDm5VsbzBjDReo3SA5h4=",
        version = "v1.13.1",
    )
    go_repository(
        name = "com_google_cloud_go_security",
        build_file_proto_mode = "disable_global",
        importpath = "cloud.google.com/go/security",
        sum = "h1:u4RCnEQPvlrrnFRFinU0T3WsjtrsQErkWBfqTM5oUQI=",
        version = "v1.17.0",
    )
    go_repository(
        name = "com_google_cloud_go_securitycenter",
        build_file_proto_mode = "disable_global",
        importpath = "cloud.google.com/go/securitycenter",
        sum = "h1:Y8C0I/mzLbaxAl5cw3EaLox0Rvpy+VUwEuCGWIQDMU8=",
        version = "v1.30.0",
    )
    go_repository(
        name = "com_google_cloud_go_servicedirectory",
        build_file_proto_mode = "disable_global",
        importpath = "cloud.google.com/go/servicedirectory",
        sum = "h1:c3OAhTcZ8LbIiKps5T3p6i0QcPI8/aWYwOfoZobICKo=",
        version = "v1.11.7",
    )
    go_repository(
        name = "com_google_cloud_go_shell",
        build_file_proto_mode = "disable_global",
        importpath = "cloud.google.com/go/shell",
        sum = "h1:HxCzcUxSsCh6FJWkmbOUrGI1sKe4E1Yy4vaykn4RhJ4=",
        version = "v1.7.7",
    )
    go_repository(
        name = "com_google_cloud_go_spanner",
        build_file_proto_mode = "disable_global",
        importpath = "cloud.google.com/go/spanner",
        sum = "h1:P6+BY70Wtol4MtryBgnXZVTZfsdySEvWfz0EpyLwHi4=",
        version = "v1.63.0",
    )
    go_repository(
        name = "com_google_cloud_go_speech",
        build_file_proto_mode = "disable_global",
        importpath = "cloud.google.com/go/speech",
        sum = "h1:TcWEAOLQH1Lb2fhHS6/GjvAh+ue0dt4xUDHXHG6vF04=",
        version = "v1.23.1",
    )
    go_repository(
        name = "com_google_cloud_go_storage",
        build_file_proto_mode = "disable_global",
        importpath = "cloud.google.com/go/storage",
        sum = "h1:B59ahL//eDfx2IIKFBeT5Atm9wnNmj3+8xG/W4WB//w=",
        version = "v1.35.1",
    )
    go_repository(
        name = "com_google_cloud_go_storagetransfer",
        build_file_proto_mode = "disable_global",
        importpath = "cloud.google.com/go/storagetransfer",
        sum = "h1:CXmoNEvz7y2NtHFZuH3Z8ASN43rxRINWa2Q/IlBzM2k=",
        version = "v1.10.6",
    )
    go_repository(
        name = "com_google_cloud_go_talent",
        build_file_proto_mode = "disable_global",
        importpath = "cloud.google.com/go/talent",
        sum = "h1:RoyEtftfJrbwJcu63zuWE4IjC76xMyVsJBhmleIp3bE=",
        version = "v1.6.8",
    )
    go_repository(
        name = "com_google_cloud_go_texttospeech",
        build_file_proto_mode = "disable_global",
        importpath = "cloud.google.com/go/texttospeech",
        sum = "h1:qR6Mu+EM2OfaZR1/Rl8BDBTVfi2X5OtwKKvJRSQyG+o=",
        version = "v1.7.7",
    )
    go_repository(
        name = "com_google_cloud_go_tpu",
        build_file_proto_mode = "disable_global",
        importpath = "cloud.google.com/go/tpu",
        sum = "h1:ngQokxUB1z2gvHn3vAf04m7SFnNYMiQIIpny81fCGAs=",
        version = "v1.6.7",
    )
    go_repository(
        name = "com_google_cloud_go_trace",
        build_file_proto_mode = "disable_global",
        importpath = "cloud.google.com/go/trace",
        sum = "h1:gK8z2BIJQ3KIYGddw9RJLne5Fx0FEXkrEQzPaeEYVvk=",
        version = "v1.10.7",
    )
    go_repository(
        name = "com_google_cloud_go_translate",
        build_file_proto_mode = "disable_global",
        importpath = "cloud.google.com/go/translate",
        sum = "h1:g+B29z4gtRGsiKDoTF+bNeH25bLRokAaElygX2FcZkE=",
        version = "v1.10.3",
    )
    go_repository(
        name = "com_google_cloud_go_video",
        build_file_proto_mode = "disable_global",
        importpath = "cloud.google.com/go/video",
        sum = "h1:YZHUgiIHE77SdZNT6gjagoni5GRcUkkrm4YCFFPqtBw=",
        version = "v1.20.6",
    )
    go_repository(
        name = "com_google_cloud_go_videointelligence",
        build_file_proto_mode = "disable_global",
        importpath = "cloud.google.com/go/videointelligence",
        sum = "h1:SKBkFTuOclESLjQL1LwraqVFm2fL5oL9tbzKITU+FOY=",
        version = "v1.11.7",
    )
    go_repository(
        name = "com_google_cloud_go_vision_v2",
        build_file_proto_mode = "disable_global",
        importpath = "cloud.google.com/go/vision/v2",
        sum = "h1:j9RxG8DcyJO/D7/ps2pOey8VZys+TMqF79bWAhuM7QU=",
        version = "v2.8.2",
    )
    go_repository(
        name = "com_google_cloud_go_vmmigration",
        build_file_proto_mode = "disable_global",
        importpath = "cloud.google.com/go/vmmigration",
        sum = "h1:bf2qKqEN7iqT62IptQ/FDadoDLJI9sthyrW3PVaH8bY=",
        version = "v1.7.7",
    )
    go_repository(
        name = "com_google_cloud_go_vmwareengine",
        build_file_proto_mode = "disable_global",
        importpath = "cloud.google.com/go/vmwareengine",
        sum = "h1:x4KwHB4JlBEzMaITVhrbbpHrU+2I5LrlvHGEEluT0vc=",
        version = "v1.1.3",
    )
    go_repository(
        name = "com_google_cloud_go_vpcaccess",
        build_file_proto_mode = "disable_global",
        importpath = "cloud.google.com/go/vpcaccess",
        sum = "h1:F5woMLufKnshmDvPVxCzoC+Di12RYXQ1W8kNmpBT8z0=",
        version = "v1.7.7",
    )
    go_repository(
        name = "com_google_cloud_go_webrisk",
        build_file_proto_mode = "disable_global",
        importpath = "cloud.google.com/go/webrisk",
        sum = "h1:EWTSVagWWeQjVAsebiF/wJMwC5bq6Zz3LqOmD9Uid4s=",
        version = "v1.9.7",
    )
    go_repository(
        name = "com_google_cloud_go_websecurityscanner",
        build_file_proto_mode = "disable_global",
        importpath = "cloud.google.com/go/websecurityscanner",
        sum = "h1:R5OW5SNRqD0DSEmyWLUMNYAXWYnz/NLSXBawVFrc9a0=",
        version = "v1.6.7",
    )
    go_repository(
        name = "com_google_cloud_go_workflows",
        build_file_proto_mode = "disable_global",
        importpath = "cloud.google.com/go/workflows",
        sum = "h1:2bE69mh68law1UZWPjgmvOQsjsGSppRudABAXwNAy58=",
        version = "v1.12.6",
    )
    go_repository(
        name = "in_gopkg_check_v1",
        build_file_proto_mode = "disable_global",
        importpath = "gopkg.in/check.v1",
        sum = "h1:Hei/4ADfdWqJk1ZMxUNpqntNwaWcugrBjAiHlqqRiVk=",
        version = "v1.0.0-20201130134442-10cb98267c6c",
    )
    go_repository(
        name = "in_gopkg_ini_v1",
        build_file_proto_mode = "disable_global",
        importpath = "gopkg.in/ini.v1",
        sum = "h1:Dgnx+6+nfE+IfzjUEISNeydPJh9AXNNsWbGP9KzCsOA=",
        version = "v1.67.0",
    )
    go_repository(
        name = "in_gopkg_op_go_logging_v1",
        build_file_proto_mode = "disable_global",
        importpath = "gopkg.in/op/go-logging.v1",
        sum = "h1:6D+BvnJ/j6e222UW8s2qTSe3wGBtvo0MbVQG/c5k8RE=",
        version = "v1.0.0-20160211212156-b2cb9fa56473",
    )
    go_repository(
        name = "in_gopkg_warnings_v0",
        build_file_proto_mode = "disable_global",
        importpath = "gopkg.in/warnings.v0",
        sum = "h1:wFXVbFY8DY5/xOe1ECiWdKCzZlxgshcYVNkBHstARME=",
        version = "v0.1.2",
    )
    go_repository(
        name = "in_gopkg_yaml_v2",
        build_file_proto_mode = "disable_global",
        importpath = "gopkg.in/yaml.v2",
        sum = "h1:D8xgwECY7CYvx+Y2n4sBz93Jn9JRvxdiyyo8CTfuKaY=",
        version = "v2.4.0",
    )
    go_repository(
        name = "in_gopkg_yaml_v3",
        build_file_proto_mode = "disable_global",
        importpath = "gopkg.in/yaml.v3",
        sum = "h1:fxVm/GzAzEWqLHuvctI91KS9hhNmmWOoWu0XTYJS7CA=",
        version = "v3.0.1",
    )
    go_repository(
        name = "io_etcd_go_etcd_api_v3",
        build_file_proto_mode = "disable_global",
        importpath = "go.etcd.io/etcd/api/v3",
        sum = "h1:W4sw5ZoU2Juc9gBWuLk5U6fHfNVyY1WC5g9uiXZio/c=",
        version = "v3.5.12",
    )
    go_repository(
        name = "io_etcd_go_etcd_client_pkg_v3",
        build_file_proto_mode = "disable_global",
        importpath = "go.etcd.io/etcd/client/pkg/v3",
        sum = "h1:EYDL6pWwyOsylrQyLp2w+HkQ46ATiOvoEdMarindU2A=",
        version = "v3.5.12",
    )
    go_repository(
        name = "io_etcd_go_etcd_client_v2",
        build_file_proto_mode = "disable_global",
        importpath = "go.etcd.io/etcd/client/v2",
        sum = "h1:0m4ovXYo1CHaA/Mp3X/Fak5sRNIWf01wk/X1/G3sGKI=",
        version = "v2.305.12",
    )
    go_repository(
        name = "io_etcd_go_etcd_client_v3",
        build_file_proto_mode = "disable_global",
        importpath = "go.etcd.io/etcd/client/v3",
        sum = "h1:v5lCPXn1pf1Uu3M4laUE2hp/geOTc5uPcYYsNe1lDxg=",
        version = "v3.5.12",
    )
    go_repository(
        name = "io_k8s_sigs_yaml",
        build_file_proto_mode = "disable_global",
        importpath = "sigs.k8s.io/yaml",
        sum = "h1:Mk1wCc2gy/F0THH0TAp1QYyJNzRm2KCLy3o5ASXVI5E=",
        version = "v1.4.0",
    )
    go_repository(
        name = "io_opencensus_go",
        build_file_proto_mode = "disable_global",
        importpath = "go.opencensus.io",
        sum = "h1:y73uSU6J157QMP2kn2r30vwW1A2W2WFwSCGnAVxeaD0=",
        version = "v0.24.0",
    )
    go_repository(
        name = "io_opentelemetry_go_contrib_instrumentation_google_golang_org_grpc_otelgrpc",
        build_file_proto_mode = "disable_global",
        importpath = "go.opentelemetry.io/contrib/instrumentation/google.golang.org/grpc/otelgrpc",
        sum = "h1:4Pp6oUg3+e/6M4C0A/3kJ2VYa++dsWVTtGgLVj5xtHg=",
        version = "v0.49.0",
    )
    go_repository(
        name = "io_opentelemetry_go_contrib_instrumentation_net_http_otelhttp",
        build_file_proto_mode = "disable_global",
        importpath = "go.opentelemetry.io/contrib/instrumentation/net/http/otelhttp",
        sum = "h1:jq9TW8u3so/bN+JPT166wjOI6/vQPF6Xe7nMNIltagk=",
        version = "v0.49.0",
    )
    go_repository(
        name = "io_opentelemetry_go_otel",
        build_file_proto_mode = "disable_global",
        importpath = "go.opentelemetry.io/otel",
        sum = "h1:0LAOdjNmQeSTzGBzduGe/rU4tZhMwL5rWgtp9Ku5Jfo=",
        version = "v1.24.0",
    )
    go_repository(
        name = "io_opentelemetry_go_otel_metric",
        build_file_proto_mode = "disable_global",
        importpath = "go.opentelemetry.io/otel/metric",
        sum = "h1:6EhoGWWK28x1fbpA4tYTOWBkPefTDQnb8WSGXlc88kI=",
        version = "v1.24.0",
    )
    go_repository(
        name = "io_opentelemetry_go_otel_trace",
        build_file_proto_mode = "disable_global",
        importpath = "go.opentelemetry.io/otel/trace",
        sum = "h1:CsKnnL4dUAr/0llH9FKuc698G04IrpWV0MQA/Y1YELI=",
        version = "v1.24.0",
    )
    go_repository(
        name = "net_starlark_go",
        build_file_proto_mode = "disable_global",
        importpath = "go.starlark.net",
        sum = "h1:YyGqCjZtGZJ+mRPaenEiB87afEO2MFRzLiJNZ0Z0bPw=",
        version = "v0.0.0-20240725214946-42030a7cedce",
    )
    go_repository(
        name = "org_golang_google_api",
        build_file_proto_mode = "disable_global",
        importpath = "google.golang.org/api",
        sum = "h1:w174hnBPqut76FzW5Qaupt7zY8Kql6fiVjgys4f58sU=",
        version = "v0.171.0",
    )
    go_repository(
        name = "org_golang_google_appengine",
        build_file_proto_mode = "disable_global",
        importpath = "google.golang.org/appengine",
        sum = "h1:IhEN5q69dyKagZPYMSdIjS2HqprW324FRQZJcGqPAsM=",
        version = "v1.6.8",
    )
    go_repository(
        name = "org_golang_google_genproto",
        build_file_proto_mode = "disable_global",
        importpath = "google.golang.org/genproto",
        sum = "h1:u7WMYrIrVvs0TF5yaKwKNbcJyySYf+HAIFXxWltJOXE=",
        version = "v0.0.0-20240528184218-531527333157",
    )
    go_repository(
        name = "org_golang_google_genproto_googleapis_api",
        build_file_proto_mode = "disable_global",
        importpath = "google.golang.org/genproto/googleapis/api",
        sum = "h1:+rdxYoE3E5htTEWIe15GlN6IfvbURM//Jt0mmkmm6ZU=",
        version = "v0.0.0-20240604185151-ef581f913117",
    )
    go_repository(
        name = "org_golang_google_genproto_googleapis_rpc",
        build_file_proto_mode = "disable_global",
        importpath = "google.golang.org/genproto/googleapis/rpc",
        sum = "h1:Zy9XzmMEflZ/MAaA7vNcoebnRAld7FsPW1EeBB7V0m8=",
        version = "v0.0.0-20240528184218-531527333157",
    )
    go_repository(
        name = "org_golang_google_grpc",
        build_file_proto_mode = "disable",
        importpath = "google.golang.org/grpc",
        sum = "h1:LKtvyfbX3UGVPFcGqJ9ItpVWW6oN/2XqTxfAnwRRXiA=",
        version = "v1.64.1",
    )
    go_repository(
        name = "org_golang_google_grpc_cmd_protoc_gen_go_grpc",
        build_file_proto_mode = "disable_global",
        importpath = "google.golang.org/grpc/cmd/protoc-gen-go-grpc",
        sum = "h1:rNBFJjBCOgVr9pWD7rs/knKL4FRTKgpZmsRfV214zcA=",
        version = "v1.3.0",
    )
    go_repository(
        name = "org_golang_google_protobuf",
        build_directives = ["gazelle:exclude **/testdata/**/*"],
        build_file_proto_mode = "disable_global",
        importpath = "google.golang.org/protobuf",
        sum = "h1:9ddQBjfCyZPOHPUiPxpYESBLc+T8P3E+Vo4IbKZgFWg=",
        version = "v1.34.1",
    )
    go_repository(
        name = "org_golang_x_crypto",
        build_file_proto_mode = "disable_global",
        importpath = "golang.org/x/crypto",
        sum = "h1:IOBPskki6Lysi0lo9qQvbxiQ+FvsCC/YWOecCHAixus=",
        version = "v0.33.0",
    )
    go_repository(
        name = "org_golang_x_exp",
        build_file_proto_mode = "disable_global",
        importpath = "golang.org/x/exp",
        sum = "h1:yqrTHse8TCMW1M1ZCP+VAR/l0kKxwaAIqN/il7x4voA=",
        version = "v0.0.0-20250106191152-7588d65b2ba8",
    )
    go_repository(
        name = "org_golang_x_mod",
        build_file_proto_mode = "disable_global",
        importpath = "golang.org/x/mod",
        sum = "h1:D4nJWe9zXqHOmWqj4VMOJhvzj7bEZg4wEYa759z1pH4=",
        version = "v0.22.0",
    )
    go_repository(
        name = "org_golang_x_net",
        build_file_proto_mode = "disable_global",
        importpath = "golang.org/x/net",
        sum = "h1:T5GQRQb2y08kTAByq9L4/bz8cipCdA8FbRTXewonqY8=",
        version = "v0.35.0",
    )
    go_repository(
        name = "org_golang_x_oauth2",
        build_file_proto_mode = "disable_global",
        importpath = "golang.org/x/oauth2",
        sum = "h1:09qnuIAgzdx1XplqJvW6CQqMCtGZykZWcXzPMPUusvI=",
        version = "v0.18.0",
    )
    go_repository(
        name = "org_golang_x_sync",
        build_file_proto_mode = "disable_global",
        importpath = "golang.org/x/sync",
        sum = "h1:GGz8+XQP4FvTTrjZPzNKTMFtSXH80RAzG+5ghFPgK9w=",
        version = "v0.11.0",
    )
    go_repository(
        name = "org_golang_x_sys",
        build_file_proto_mode = "disable_global",
        importpath = "golang.org/x/sys",
        sum = "h1:QjkSwP/36a20jFYWkSue1YwXzLmsV5Gfq7Eiy72C1uc=",
        version = "v0.30.0",
    )
    go_repository(
        name = "org_golang_x_term",
        build_file_proto_mode = "disable_global",
        importpath = "golang.org/x/term",
        sum = "h1:L6pJp37ocefwRRtYPKSWOWzOtWSxVajvz2ldH/xi3iU=",
        version = "v0.29.0",
    )
    go_repository(
        name = "org_golang_x_text",
        build_file_proto_mode = "disable_global",
        importpath = "golang.org/x/text",
        sum = "h1:bofq7m3/HAFvbF51jz3Q9wLg3jkvSPuiZu/pD1XwgtM=",
        version = "v0.22.0",
    )
    go_repository(
        name = "org_golang_x_time",
        build_file_proto_mode = "disable_global",
        importpath = "golang.org/x/time",
        sum = "h1:o7cqy6amK/52YcAKIPlM3a+Fpj35zvRj2TP+e1xFSfk=",
        version = "v0.5.0",
    )
    go_repository(
        name = "org_golang_x_tools",
        build_directives = ["gazelle:exclude **/testdata/**/*"],
        build_file_proto_mode = "disable_global",
        importpath = "golang.org/x/tools",
        sum = "h1:Xx0h3TtM9rzQpQuR4dKLrdglAmCEN5Oi+P74JdhdzXE=",
        version = "v0.29.0",
    )
    go_repository(
        name = "org_golang_x_tools_go_vcs",
        build_file_proto_mode = "disable_global",
        importpath = "golang.org/x/tools/go/vcs",
        sum = "h1:cOIJqWBl99H1dH5LWizPa+0ImeeJq3t3cJjaeOWUAL4=",
        version = "v0.1.0-deprecated",
    )
    go_repository(
        name = "org_golang_x_xerrors",
        build_file_proto_mode = "disable_global",
        importpath = "golang.org/x/xerrors",
        sum = "h1:H2TDz8ibqkAF6YGhCdN3jS9O0/s90v0rJh3X/OLHEUk=",
        version = "v0.0.0-20220907171357-04be3eba64a2",
    )
    go_repository(
        name = "org_uber_go_atomic",
        build_file_proto_mode = "disable_global",
        importpath = "go.uber.org/atomic",
        sum = "h1:ECmE8Bn/WFTYwEW/bpKD3M8VtR/zQVbavAoalC1PYyE=",
        version = "v1.9.0",
    )
    go_repository(
        name = "org_uber_go_multierr",
        build_file_proto_mode = "disable_global",
        importpath = "go.uber.org/multierr",
        sum = "h1:7fIwc/ZtS0q++VgcfqFDxSBZVv/Xo49/SYnDFupUwlI=",
        version = "v1.9.0",
    )
    go_repository(
        name = "org_uber_go_zap",
        build_file_proto_mode = "disable_global",
        importpath = "go.uber.org/zap",
        sum = "h1:WefMeulhovoZ2sYXz7st6K0sLj7bBhpiFaud4r4zST8=",
        version = "v1.21.0",
    )
