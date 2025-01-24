package starlark

import (
	"github.com/bazelbuild/bazel-gazelle/rule"
	"github.com/bazelbuild/buildtools/build"
)

// Utils similar-to/extending gazelle rule.* utils

func AttrBool(r *rule.Rule, attr string) bool {
	if v := r.Attr(attr); v != nil {
		if i, isIdent := v.(*build.Ident); isIdent {
			return i.Name == "True"
		}
	}

	return false
}

func AttrMap(r *rule.Rule, attr string) []*build.KeyValueExpr {
	if v := r.Attr(attr); v != nil {
		if dict, isDict := v.(*build.DictExpr); isDict {
			return dict.List
		}
	}

	return []*build.KeyValueExpr{}
}
