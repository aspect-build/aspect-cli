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
	"path"
	"path/filepath"
	"strings"

	"go.starlark.net/repl"
	"go.starlark.net/starlark"
	"go.starlark.net/syntax"

	// filepathx supports double-star glob patterns (the stdlib doesn't). This
	// is necessary to match the behaviour from Bazel.
	"github.com/yargevad/filepathx"

	common "aspect.build/cli/gazelle/common"
	BazelLog "aspect.build/cli/pkg/logger"
	"github.com/bazelbuild/bazel-gazelle/config"
	bzl "github.com/bazelbuild/buildtools/build"
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

func ExpandSrcs(repoRoot, pkg string, expr bzl.Expr) ([]string, error) {
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

	// Glob expression that must be evaluated
	thread := &starlark.Thread{Load: expandSrcsLoadOptions}
	globber := Globber{
		repoRoot: repoRoot,
		pkg:      pkg,
	}
	env := starlark.StringDict{"glob": starlark.NewBuiltin("glob", globber.Glob)}
	srcsSyntaxExpr, err := expandSrcsFileOptions.ParseExpr("", bzl.FormatString(expr), 0)
	if err != nil {
		return nil, fmt.Errorf("Expression parse error: %w", err)
	}
	srcsVal, err := starlark.EvalExprOptions(expandSrcsFileOptions, thread, srcsSyntaxExpr, env)
	if err != nil {
		return nil, fmt.Errorf("Expression evaluation error: %w", err)
	}
	srcsValList := srcsVal.(*starlark.List)
	srcs := make([]string, 0, srcsValList.Len())
	srcsValListIterator := srcsValList.Iterate()
	var srcVal starlark.Value
	for srcsValListIterator.Next(&srcVal) {
		src := srcVal.(starlark.String)
		srcs = append(srcs, string(src))
	}
	return srcs, nil
}

// Globber implements the glob built-in to evaluate the srcs attribute containing glob patterns.
type Globber struct {
	repoRoot string
	pkg      string
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
	absPkg := path.Join(g.repoRoot, g.pkg)
	var includeArg starlark.Value
	if len(args) == 1 {
		includeArg = args[0]
	}
	var excludeArg starlark.Value
	allowEmpty := true
	for _, kwarg := range kwargs {
		switch kwarg[0] {
		case starlark.String("include"):
			if includeArg != nil {
				return nil, fmt.Errorf("failed glob: invalid syntax: cannot use include as kwarg and arg")
			}
			includeArg = kwarg[1]
		case starlark.String("exclude"):
			excludeArg = kwarg[1]
		case starlark.String("exclude_directories"):
			excludeDirectoriesArg := kwarg[1]
			excludeDirectoriesInt, ok := excludeDirectoriesArg.(starlark.Int)
			if !ok {
				return nil, fmt.Errorf("failed glob: invalid syntax: exclude_directories must be 0 or 1")
			}
			excludeDirectories, ok := excludeDirectoriesInt.Int64()
			if !ok || (excludeDirectories != 0 && excludeDirectories != 1) {
				return nil, fmt.Errorf("failed glob: invalid syntax: exclude_directories must be 0 or 1")
			}
			// TODO: implement.
			BazelLog.Warnf("WARNING: the 'exclude_directories' attribute of 'glob' was set but is not supported by Gazelle")
		case starlark.String("allow_empty"):
			allowEmptyArg := kwarg[1]
			allowEmptyAssert, ok := allowEmptyArg.(starlark.Bool)
			if !ok {
				return nil, fmt.Errorf("failed glob: invalid syntax: allow_empty must be a boolean")
			}
			allowEmpty = bool(allowEmptyAssert)
		default:
			return nil, fmt.Errorf("failed glob: invalid syntax: kwarg %q not recognized", kwarg[0])
		}
	}

	excludeSet := make(map[string]struct{})
	if excludeArg != nil {
		excludePatterns, ok := excludeArg.(*starlark.List)
		if !ok {
			return nil, fmt.Errorf("failed glob: exclude is not a list")
		}
		excludeIterator := excludePatterns.Iterate()
		var excludePatternVal starlark.Value
		for excludeIterator.Next(&excludePatternVal) {
			excludePattern, ok := excludePatternVal.(starlark.String)
			if !ok {
				return nil, fmt.Errorf("failed glob: exclude pattern must be a string")
			}
			absPattern := path.Join(absPkg, string(excludePattern))
			matches, err := filepathx.Glob(absPattern)
			if err != nil {
				return nil, fmt.Errorf("failed glob: %w", err)
			}
			for _, match := range matches {
				exclude, _ := filepath.Rel(absPkg, match)
				excludeSet[exclude] = struct{}{}
			}
		}
	}

	rootBazelPackageTree := NewBazelPackageTree(g.pkg)
	includePatterns, ok := includeArg.(*starlark.List)
	if !ok {
		return nil, fmt.Errorf("failed glob: include is not a list")
	}
	includeIterator := includePatterns.Iterate()
	var includePatternVal starlark.Value
	for includeIterator.Next(&includePatternVal) {
		includePattern, ok := includePatternVal.(starlark.String)
		if !ok {
			return nil, fmt.Errorf("failed glob: include pattern must be a string")
		}
		absPattern := path.Join(absPkg, string(includePattern))
		matches, err := filepathx.Glob(absPattern)
		if err != nil {
			return nil, fmt.Errorf("failed glob: %w", err)
		}
		for _, match := range matches {
			src, _ := filepath.Rel(absPkg, match)
			if _, excluded := excludeSet[src]; !excluded {
				parts := strings.Split(src, string(filepath.Separator))
				rootBazelPackageTree.AddPath(parts)
			}
		}
	}

	result := rootBazelPackageTree.Paths()

	if !allowEmpty && len(result) == 0 {
		return nil, fmt.Errorf("failed glob: 'allow_empty' was set and the result was empty")
	}

	return starlark.NewList(result), nil
}

// BazelPackageTree is a representation of a filesystem tree specialized for
// filtering paths that are under a Bazel sub-package. It understands the
// file-based boundaries that represent a sub-package (a nested BUILD file).
// The nature of this data structure also enables us to remove duplicated paths.
type BazelPackageTree struct {
	// pkg is the Bazel package this tree represents.
	pkg *string
	// branches is the connected branches of this tree, which is a recursive
	// field.
	branches map[string]*BazelPackageTree
	// isBazelPackage indicates whether this tree (which can also be considered
	// a "node" in the whole tree) is a Bazel package or not. This is used to
	// filter out sub-packages.
	isBazelPackage bool
	// isFile indicates whether this node is a leaf or not, so, when returning
	// the list of paths, we know append the part without joining it to the
	// child branches. This also enables constructing the paths without
	// returning partial paths during the recursion.
	isFile bool
}

// NewBazelPackageTree constructs a new BazelPackageTree.
func NewBazelPackageTree(pkg string) *BazelPackageTree {
	return &BazelPackageTree{
		pkg:      &pkg,
		branches: make(map[string]*BazelPackageTree),
	}
}

// AddPath adds a path to the package tree.
func (pt *BazelPackageTree) AddPath(parts []string) {
	branches := pt.branches
	for i, part := range parts {
		branch, exists := branches[part]
		if !exists {
			isFile := (i == len(parts)-1)
			var isBazelPkg bool
			if !isFile {
				dir := path.Join(parts[:i+1]...)
				dir = path.Join(*pt.pkg, dir)
				isBazelPkg = common.HasBUILDFile(config.DefaultValidBuildFileNames, dir)
			}
			branch = &BazelPackageTree{
				pkg:            pt.pkg,
				branches:       make(map[string]*BazelPackageTree),
				isBazelPackage: isBazelPkg,
				isFile:         isFile,
			}
			branches[part] = branch
		}
		branches = branch.branches
	}
}

// Paths returns the list of paths in the tree, filtering Bazel sub-packages.
func (pt *BazelPackageTree) Paths() []starlark.Value {
	paths := make([]starlark.Value, 0)
	for part, branch := range pt.branches {
		if branch.isBazelPackage {
			continue
		}
		if branch.isFile {
			paths = append(paths, starlark.String(part))
		}
		for _, branchPath := range branch.Paths() {
			paths = append(paths, starlark.String(path.Join(part, string(branchPath.(starlark.String)))))
		}
	}
	return paths
}
