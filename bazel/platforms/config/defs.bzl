"""This module generated the platforms list for the build matrix."""

def linker_suffix(delimiter, linker):
    if linker == "unknown":
        return ""
    return delimiter + linker

cpus = ["aarch64", "x86_64"]
os_to_linker = {
    "linux": ["musl", "unknown"],
    "macos": ["unknown"],
}

platforms = [
    struct(os = os, cpu = cpu, linker = linker)
    for os in os_to_linker
    for linker in os_to_linker[os]
    for cpu in cpus
]
