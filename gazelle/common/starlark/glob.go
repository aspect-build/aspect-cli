/* Skylark glob implementation in Go.
 *
 * Based on initial work by Thulio Ferraz Assis (https://github.com/f0rmiga):
 *   https://github.com/bazelbuild/rules_python/commit/b72cdff5d3f65a71d8c08c51097f7e49de9157a6
 *   https://github.com/bazelbuild/rules_python/commit/e8daf9e4f3c818d451c66490e43044a20edb5267
 *   https://github.com/bazelbuild/rules_python/commit/70066a1f585e2ae6d1013cab9f47bf45c5e7d1ae
 */

package starlark

import (
	"fmt"

	"go.starlark.net/repl"
	"go.starlark.net/starlark"
	"go.starlark.net/syntax"

	BazelLog "github.com/aspect-build/aspect-cli/pkg/logger"
	bzl "github.com/bazelbuild/buildtools/build"
	"github.com/bmatcuk/doublestar/v4"
)

func IsCustomSrcs(srcs bzl.Expr) bool {
	_, ok := srcs.(*bzl.ListExpr)
	return !ok
}

// FileOptions for evaluating glob expressions
var expandSrcsFileOptions = &syntax.FileOptions{
	Set:               false,
	While:             false,
	TopLevelControl:   false,
	GlobalReassign:    false,
	LoadBindsGlobally: false,
	Recursion:         false,
}

// LoaderOptions for evaluating glob expressions
var expandSrcsLoadOptions = repl.MakeLoadOptions(expandSrcsFileOptions)

func ExpandSrcs(repoRoot, pkg string, files []string, expr bzl.Expr) ([]string, error) {
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

	// Starlark thread+env for evaluating the expression
	thread := &starlark.Thread{Load: expandSrcsLoadOptions}
	globber := Globber{files: files}
	env := starlark.StringDict{"glob": starlark.NewBuiltin("glob", globber.Glob)}

	// Parse the expression
	srcsSyntaxExpr, err := expandSrcsFileOptions.ParseExpr("", bzl.FormatString(expr), 0)
	if err != nil {
		return nil, fmt.Errorf("Expression parse error: %w", err)
	}

	// Evaluate the expression
	srcsVal, err := starlark.EvalExprOptions(expandSrcsFileOptions, thread, srcsSyntaxExpr, env)
	if err != nil {
		return nil, fmt.Errorf("Expression evaluation error: %w", err)
	}

	srcsValList := srcsVal.(*starlark.List)
	srcs := make([]string, 0, srcsValList.Len())
	for src := range srcsValList.Elements() {
		srcs = append(srcs, string(src.(starlark.String)))
	}
	return srcs, nil
}

// Globber implements the glob built-in to evaluate the srcs attribute containing glob patterns.
type Globber struct {
	files []string
}

func parseGlobArgs(args starlark.Tuple, kwargs []starlark.Tuple) ([]string, []string, bool, error) {
	var includeArg starlark.Value = nil
	var excludeArg starlark.Value = nil
	var allowEmpty starlark.Bool = starlark.False

	if len(args) == 1 {
		includeArg = args[0]
	}
	for _, kwarg := range kwargs {
		switch kwarg[0] {
		case starlark.String("include"):
			if includeArg != nil {
				return nil, nil, false, fmt.Errorf("invalid syntax: cannot use include as kwarg and arg")
			}
			includeArg = kwarg[1]
		case starlark.String("exclude"):
			excludeArg = kwarg[1]
		case starlark.String("exclude_directories"):
			// TODO: implement.
			BazelLog.Warnf("WARNING: the 'exclude_directories' attribute of 'glob' was set but is not supported by Gazelle")

		case starlark.String("allow_empty"):
			allowEmptyAssert, ok := kwarg[1].(starlark.Bool)
			if !ok {
				return nil, nil, false, fmt.Errorf("invalid syntax: allow_empty must be a boolean")
			}
			allowEmpty = allowEmptyAssert
		default:
			return nil, nil, false, fmt.Errorf("invalid syntax: kwarg %q not recognized", kwarg[0])
		}
	}

	// An include array is required
	if includeArg == nil {
		return nil, nil, false, fmt.Errorf("include is required")
	}

	// Convert + assert to include/exclude lists
	includePatterns, ok := includeArg.(*starlark.List)
	if !ok {
		return nil, nil, false, fmt.Errorf("include must be a List")
	}
	excludePatterns, ok := excludeArg.(*starlark.List)
	if excludeArg != nil && !ok {
		return nil, nil, false, fmt.Errorf("exclude must be a List")
	}

	// Convert to string slices + warn on other types
	var includeA, excludeA []string

	includeA = make([]string, 0, includePatterns.Len())
	for includeP := range includePatterns.Elements() {
		if s, ok := includeP.(starlark.String); ok {
			includeA = append(includeA, string(s))
		} else {
			BazelLog.Warnf("WARNING: invalid glob include type %T", includeP)
		}
	}

	if excludePatterns != nil {
		excludeA = make([]string, 0, excludePatterns.Len())
		for excludeP := range excludePatterns.Elements() {
			if s, ok := excludeP.(starlark.String); ok {
				excludeA = append(excludeA, string(s))
			} else {
				BazelLog.Warnf("WARNING: invalid glob exclude type %T", excludeP)
			}
		}
	}

	return includeA, excludeA, bool(allowEmpty), nil
}

// Glob expands the glob patterns and filters Bazel sub-packages from the tree.
// This is used to index manually created targets that contain globs so the
// resolution phase depends less on `gazelle:resolve` directives set by the
// user.
func (g *Globber) Glob(
	_ *starlark.Thread,
	_ *starlark.Builtin,
	args starlark.Tuple,
	kwargs []starlark.Tuple,
) (starlark.Value, error) {
	if len(args) > 1 {
		return nil, fmt.Errorf("failed glob: only 1 positional argument is allowed")
	}

	include, exclude, allowEmpty, err := parseGlobArgs(args, kwargs)
	if err != nil {
		return nil, err
	}

	matches := []starlark.Value{}

	for _, file := range g.files {
		matched := false

		for _, pattern := range include {
			if doublestar.MatchUnvalidated(pattern, file) {
				matched = true
				break
			}
		}

		if matched {
			for _, pattern := range exclude {
				if doublestar.MatchUnvalidated(pattern, file) {
					matched = false
					break
				}
			}
		}

		if matched {
			matches = append(matches, starlark.String(file))
		}
	}

	if !allowEmpty && len(matches) == 0 {
		return nil, fmt.Errorf("no files matched the glob pattern")
	}

	return starlark.NewList(matches), nil
}
