package gazelle

import (
	"github.com/bazelbuild/bazel-gazelle/resolve"
	"github.com/emirpasic/gods/sets/treeset"
	godsutils "github.com/emirpasic/gods/utils"
)

// ImportStatement represents an ImportSpec imported from a source file.
// Imports can be of any form (es6, cjs, amd, ...).
// Imports may be relative ot the source, absolute, workspace, named modules etc.
type ImportStatement struct {
	resolve.ImportSpec

	// Alternative paths this statement may resolve to
	Alt []string

	// The path of the file containing the import
	SourcePath string

	// The path as written in the import statement
	ImportPath string
}

// NpmPackage rule import data
type NpmPackageImports struct{}

func newNpmPackageImports() *NpmPackageImports {
	return &NpmPackageImports{}
}

// Npm link-all rule import data
type LinkAllPackagesImports struct{}

func newLinkAllPackagesImports() *LinkAllPackagesImports {
	return &LinkAllPackagesImports{}
}

// TsProject rule import data
type TsProjectImports struct {
	imports *treeset.Set
}

func newTsProjectImports() *TsProjectImports {
	return &TsProjectImports{
		imports: treeset.NewWith(importStatementComparator),
	}
}
func (i *TsProjectImports) Add(impt ImportStatement) {
	i.imports.Add(impt)
}

// importStatementComparator compares modules by name.
func importStatementComparator(a, b interface{}) int {
	return godsutils.StringComparator(a.(ImportStatement).Imp, b.(ImportStatement).Imp)
}
