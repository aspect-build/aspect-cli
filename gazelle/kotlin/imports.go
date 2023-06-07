package gazelle

import (
	"github.com/bazelbuild/bazel-gazelle/resolve"
	"github.com/emirpasic/gods/sets/treeset"
	godsutils "github.com/emirpasic/gods/utils"
)

type ImportStatement struct {
	resolve.ImportSpec

	// The path of the file containing the import
	SourcePath string
}

// TsProject rule import data
type KotlinImports struct {
	imports *treeset.Set
}

func newKotlinImports() *KotlinImports {
	return &KotlinImports{
		imports: treeset.NewWith(importStatementComparator),
	}
}
func (i *KotlinImports) Add(impt ImportStatement) {
	i.imports.Add(impt)
}

// importStatementComparator compares modules by name.
func importStatementComparator(a, b interface{}) int {
	return godsutils.StringComparator(a.(ImportStatement).Imp, b.(ImportStatement).Imp)
}
