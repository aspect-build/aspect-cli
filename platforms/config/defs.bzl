"""This module generated the platforms list for the build matrix."""

oses = ["macos", "linux"]
cpus = ["aarch64", "x86_64"]

platforms = [
    struct(os = os, cpu = cpu)
    for os in oses
    for cpu in cpus
]
