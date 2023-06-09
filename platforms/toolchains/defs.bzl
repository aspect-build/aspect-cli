"""This module registers all the LLVM toolchains that we want to use."""

execution_oses = ["macos", "linux"]
execution_cpus = ["aarch64", "x86_64"]
target_oses = ["macos", "linux"]
target_cpus = ["aarch64", "x86_64"]

platforms = [
    struct(exe_os = exe_os, exe_cpu = exe_cpu, tgt_os = tgt_os, tgt_cpu = tgt_cpu)
    for exe_os in execution_oses
    for exe_cpu in execution_cpus
    for tgt_os in target_oses
    for tgt_cpu in target_cpus
]

# buildifier: disable=unnamed-macro
def register_llvm_toolchains():
    for p in platforms:
        native.register_toolchains("//platforms/toolchains:{}_{}_{}_{}_llvm".format(p.exe_os, p.exe_cpu, p.tgt_os, p.tgt_cpu))
