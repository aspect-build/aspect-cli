"""
Macros for invoking postcss.
"""

def postcss(name, config, srcs):
    # NOTE: should actually invoke the postcss package binary
    filegroup(name = name, srcs = [config] + srcs)
