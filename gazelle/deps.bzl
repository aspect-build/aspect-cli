"""Utils for fetching aspect gazelle dependencies"""

load("//gazelle/common/treesitter/grammars:grammars.bzl", _fetch_grammars = "fetch_grammars")

fetch_grammars = _fetch_grammars

def fetch_deps():
    _fetch_grammars()
