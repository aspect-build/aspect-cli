package starlark

import (
	"fmt"

	BazelLog "github.com/aspect-build/aspect-cli/gazelle/common/logger"
	"github.com/bazelbuild/bazel-gazelle/rule"
	bzl "github.com/bazelbuild/buildtools/build"
	"github.com/bmatcuk/doublestar/v4"
)

func IsCustomSrcs(srcs bzl.Expr) bool {
	_, ok := srcs.(*bzl.ListExpr)
	return !ok
}

func ExpandSrcs(files []string, expr bzl.Expr) ([]string, error) {
	// Pure array of source paths.
	if list, ok := expr.(*bzl.ListExpr); ok {
		srcs := make([]string, 0, len(list.List))
		for _, e := range list.List {
			if str, ok := e.(*bzl.StringExpr); ok {
				srcs = append(srcs, str.Value)
			} else {
				BazelLog.Tracef("skipping non-string src %s", e)
			}
		}
		return srcs, nil
	}

	g, isGlob := rule.ParseGlobExpr(expr)
	if !isGlob {
		return nil, fmt.Errorf("expected glob expression, got %s", expr)
	}

	matches := []string{}

	for _, file := range files {
		matched := false

		for _, pattern := range g.Patterns {
			if doublestar.MatchUnvalidated(pattern, file) {
				matched = true
				break
			}
		}

		if matched {
			for _, pattern := range g.Excludes {
				if doublestar.MatchUnvalidated(pattern, file) {
					matched = false
					break
				}
			}
		}

		if matched {
			matches = append(matches, file)
		}
	}

	return matches, nil
}
